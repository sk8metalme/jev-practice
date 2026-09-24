//! jevxが `$JEVX_HOME` に作るローカルデータの一覧・書き出し・削除。
//!
//! 対象はjevxが自分で書き込むファイルだけで、利用者が作る `.jevx/compact-context.md` や
//! Codexの `hooks.json` とそのbackupには触れない。

use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

use fs2::FileExt;
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::JevxError;
use crate::compact_assist::COMPACT_ASSIST_LOCK_FILE_NAME;
use crate::hook_dedupe::{DEDUPE_LOCK_FILE_NAME, DEDUPE_TEMP_DIR_NAME};

pub const DATA_SCHEMA_VERSION: u8 = 7;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DataFile {
    pub kind: &'static str,
    pub path: PathBuf,
    pub exists: bool,
    pub bytes: u64,
    pub records: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DataInventory {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    #[serde(rename = "dataHome")]
    pub data_home: PathBuf,
    pub files: Vec<DataFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PurgeReport {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    #[serde(rename = "dryRun")]
    pub dry_run: bool,
    pub removed: Vec<PathBuf>,
}

/// jevxが書き込むファイルの種類と、`$JEVX_HOME` からの相対パス。
const MANAGED_FILES: [(&str, &str); 10] = [
    ("telemetry", "events.jsonl"),
    ("hookRecords", "hooks.jsonl"),
    ("hookDedupe", "hook-dedupe.json"),
    ("hookDedupeLock", "hook-dedupe.lock"),
    ("hookDedupeTemps", ".hook-dedupe-tmp"),
    ("compactionLock", "compaction/.compact-assist.lock"),
    ("compactionRecords", "compaction/hook-records.jsonl"),
    ("compactionCheckpoints", "compaction/checkpoints.jsonl"),
    ("decisionReceipts", "decisions.jsonl"),
    ("reviewReceipts", "reviews.jsonl"),
];
const MANAGED_DIR: &str = "compaction";
#[derive(Debug, Clone)]
struct DedupeTempEntry {
    name: std::ffi::OsString,
    path: PathBuf,
    bytes: u64,
    is_regular_file: bool,
    is_symlink: bool,
}
#[cfg(unix)]
type CompactionDirectoryHandle = File;
#[cfg(not(unix))]
type CompactionDirectoryHandle = ();

pub fn data_inventory(data_home: &Path) -> Result<DataInventory, JevxError> {
    let compaction_dir = data_home.join(MANAGED_DIR);
    let compaction_dir_is_real = fs::symlink_metadata(&compaction_dir)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink());
    let files = MANAGED_FILES
        .iter()
        .map(|(kind, relative)| {
            let path = data_home.join(relative);
            if relative.starts_with("compaction/") && !compaction_dir_is_real {
                return DataFile {
                    kind,
                    path,
                    exists: false,
                    bytes: 0,
                    records: 0,
                };
            }
            // symlink_metadata: 最後の要素がリンクでもリンク自体を管理対象として扱う。
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                return DataFile {
                    kind,
                    path,
                    exists: false,
                    bytes: 0,
                    records: 0,
                };
            };
            let (bytes, records) = if metadata.is_dir() {
                if *kind == "hookDedupeTemps" {
                    dedupe_temp_directory_stats(&path)
                } else {
                    directory_stats(&path)
                }
            } else if metadata.is_file() {
                let records = read_managed_file(data_home, &path)
                    .map_or(0, |bytes| count_nonempty_jsonl_lines(&bytes));
                (metadata.len(), records)
            } else {
                // Links and special files are listed as managed entries but never opened.
                (metadata.len(), 0)
            };
            DataFile {
                kind,
                exists: true,
                bytes,
                records,
                path,
            }
        })
        .collect();
    Ok(DataInventory {
        schema_version: DATA_SCHEMA_VERSION,
        data_home: data_home.to_path_buf(),
        files,
    })
}

/// 保存済みのJSONLをkindごとの配列として返す。壊れた行は内容を出さずに位置だけ報告する。
pub fn export_data(inventory: &DataInventory) -> Result<Value, JevxError> {
    let mut records = Map::new();
    for file in &inventory.files {
        if file.kind == "hookDedupeTemps" {
            records.insert(
                file.kind.to_owned(),
                Value::Array(dedupe_temp_metadata(&file.path)?),
            );
            continue;
        }
        let mut values = Vec::new();
        if file.exists {
            let content =
                String::from_utf8_lossy(&read_managed_file(&inventory.data_home, &file.path)?)
                    .into_owned();
            for (index, line) in content.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let value = serde_json::from_str(line).map_err(|_| {
                    JevxError::InvalidInput(format!(
                        "{}:{} is not valid JSON",
                        file.path.display(),
                        index + 1
                    ))
                })?;
                values.push(value);
            }
        }
        records.insert(file.kind.to_owned(), Value::Array(values));
    }
    Ok(json!({
        "schemaVersion": DATA_SCHEMA_VERSION,
        "dataHome": inventory.data_home,
        "records": records,
    }))
}

/// `confirmed` がfalseなら削除対象を返すだけ。実際に対象を削除する場合は、ロック下で空のtemp directoryも片付ける。
pub fn purge_data(inventory: &DataInventory, confirmed: bool) -> Result<PurgeReport, JevxError> {
    let (removed, dry_run) = if confirmed {
        let current_inventory = data_inventory(&inventory.data_home)?;
        let current_targets = purge_targets(&current_inventory)?;
        let removed = if current_targets.is_empty() {
            validate_existing_compaction_paths(&inventory.data_home)?;
            Vec::new()
        } else {
            with_purge_locks(&inventory.data_home, |directory| {
                purge_current_inventory(&inventory.data_home, directory)
            })?
        };
        (removed, false)
    } else {
        (purge_targets(inventory)?, true)
    };
    Ok(PurgeReport {
        schema_version: DATA_SCHEMA_VERSION,
        dry_run,
        removed,
    })
}

fn validate_existing_compaction_paths(data_home: &Path) -> Result<(), JevxError> {
    let compaction_dir = data_home.join(MANAGED_DIR);
    match fs::symlink_metadata(&compaction_dir) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(JevxError::InvalidInput(
                "compaction directory must be a real directory".to_owned(),
            ));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }

    let lock_path = compaction_dir.join(COMPACT_ASSIST_LOCK_FILE_NAME);
    match fs::symlink_metadata(lock_path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(JevxError::InvalidInput(
            "compaction lock must be a regular file".to_owned(),
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn purge_targets(inventory: &DataInventory) -> Result<Vec<PathBuf>, JevxError> {
    let mut removed = Vec::new();
    for file in &inventory.files {
        if !file.exists || matches!(file.kind, "hookDedupeLock" | "compactionLock") {
            continue;
        }
        if file.kind == "hookDedupeTemps" {
            let metadata = match fs::symlink_metadata(&file.path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                // The temp directory itself is a managed entry if it is replaced by a link
                // or another file type; purge removes that entry without traversing it.
                removed.push(file.path.clone());
            } else {
                removed.extend(dedupe_temp_paths(&file.path)?);
            }
        } else {
            removed.push(file.path.clone());
        }
    }
    Ok(removed)
}

fn purge_current_inventory(
    data_home: &Path,
    locked_directory: &CompactionDirectoryHandle,
) -> Result<Vec<PathBuf>, JevxError> {
    ensure_locked_compaction_directory(data_home, locked_directory)?;
    let inventory = data_inventory(data_home)?;
    ensure_locked_compaction_directory(data_home, locked_directory)?;
    let removed = purge_inventory(&inventory, locked_directory)?;
    ensure_locked_compaction_directory(data_home, locked_directory)?;
    Ok(removed)
}

fn purge_inventory(
    inventory: &DataInventory,
    locked_directory: &CompactionDirectoryHandle,
) -> Result<Vec<PathBuf>, JevxError> {
    let mut removed = Vec::new();
    for file in &inventory.files {
        if !file.exists || matches!(file.kind, "hookDedupeLock" | "compactionLock") {
            continue;
        }
        if file.kind == "hookDedupeTemps" {
            let metadata = match fs::symlink_metadata(&file.path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                remove_managed_entry(inventory, &file.path, locked_directory, &mut removed)?;
                continue;
            }
            #[cfg(unix)]
            {
                // Keep the opened directory alive through enumeration and unlinkat so a
                // replacement of `.hook-dedupe-tmp` cannot redirect deletion elsewhere.
                let directory = open_dedupe_temp_directory(&file.path)?;
                let entries = dedupe_temp_entries_from_handle(&file.path, &directory)?;
                removed.extend(purge_dedupe_temp_entries(&directory, entries)?);
            }
            #[cfg(not(unix))]
            {
                for entry in dedupe_temp_entries(&file.path)? {
                    if entry.is_regular_file || entry.is_symlink {
                        remove_managed_entry(
                            inventory,
                            &entry.path,
                            locked_directory,
                            &mut removed,
                        )?;
                    }
                }
            }
        } else {
            remove_managed_entry(inventory, &file.path, locked_directory, &mut removed)?;
        }
    }
    cleanup_empty_dedupe_temp_directory(&inventory.data_home)?;
    Ok(removed)
}

fn remove_managed_entry(
    inventory: &DataInventory,
    path: &Path,
    locked_directory: &CompactionDirectoryHandle,
    removed: &mut Vec<PathBuf>,
) -> Result<(), JevxError> {
    removed.push(path.to_path_buf());
    match remove_managed_file(inventory, path, locked_directory) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn cleanup_empty_dedupe_temp_directory(data_home: &Path) -> Result<(), JevxError> {
    let path = data_home.join(DEDUPE_TEMP_DIR_NAME);
    #[cfg(unix)]
    {
        let directory = match open_dedupe_temp_directory(&path) {
            Ok(directory) => directory,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        if !dedupe_temp_entries_from_handle(&path, &directory)?.is_empty() {
            return Ok(());
        }
        if !ensure_dedupe_temp_directory_identity(&path, &directory)? {
            return Ok(());
        }
    }
    #[cfg(not(unix))]
    {
        if !fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_dir())
            || fs::read_dir(&path)?.next().is_some()
        {
            return Ok(());
        }
    }
    remove_empty_directory(&path).map_err(Into::into)
}

#[cfg(unix)]
fn ensure_dedupe_temp_directory_identity(path: &Path, directory: &File) -> Result<bool, JevxError> {
    let path_metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let directory_metadata = directory.metadata()?;
    if !path_metadata.is_dir()
        || path_metadata.file_type().is_symlink()
        || path_metadata.dev() != directory_metadata.dev()
        || path_metadata.ino() != directory_metadata.ino()
    {
        return Err(JevxError::InvalidInput(
            "dedupe temp directory changed while purge was running".to_owned(),
        ));
    }
    Ok(true)
}

fn remove_empty_directory(path: &Path) -> io::Result<()> {
    match fs::remove_dir(path) {
        Err(error)
            if error.kind() == ErrorKind::NotFound
                || error.kind() == ErrorKind::DirectoryNotEmpty =>
        {
            Ok(())
        }
        result => result,
    }
}

#[cfg(unix)]
fn ensure_locked_compaction_directory(
    data_home: &Path,
    locked_directory: &CompactionDirectoryHandle,
) -> Result<(), JevxError> {
    let path = data_home.join(MANAGED_DIR);
    let path_metadata = fs::symlink_metadata(path)?;
    let directory_metadata = locked_directory.metadata()?;
    if !path_metadata.is_dir()
        || path_metadata.file_type().is_symlink()
        || path_metadata.dev() != directory_metadata.dev()
        || path_metadata.ino() != directory_metadata.ino()
    {
        return Err(JevxError::InvalidInput(
            "compaction directory changed while purge lock was held".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_locked_compaction_directory(
    _data_home: &Path,
    _locked_directory: &CompactionDirectoryHandle,
) -> Result<(), JevxError> {
    Ok(())
}

fn remove_managed_file(
    inventory: &DataInventory,
    path: &Path,
    locked_directory: &CompactionDirectoryHandle,
) -> io::Result<()> {
    let compaction_dir = inventory.data_home.join(MANAGED_DIR);
    let dedupe_temp_dir = inventory.data_home.join(DEDUPE_TEMP_DIR_NAME);
    if path.parent() == Some(dedupe_temp_dir.as_path()) {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "dedupe temp children require directory-relative deletion",
        ));
    }
    if path.parent() == Some(compaction_dir.as_path()) {
        #[cfg(unix)]
        {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    io::Error::new(ErrorKind::InvalidInput, "invalid managed filename")
                })?;
            let name = std::ffi::CString::new(name)
                .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "invalid managed filename"))?;
            // SAFETY: `locked_directory` owns a directory fd and `name` is NUL-terminated.
            // unlinkat removes the named entry itself and never follows a final symlink.
            let result = unsafe { libc::unlinkat(locked_directory.as_raw_fd(), name.as_ptr(), 0) };
            return if result == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            };
        }
    }
    fs::remove_file(path)
}

#[cfg(unix)]
fn unlink_dedupe_temp_entry(directory: &File, name: &std::ffi::OsStr) -> io::Result<()> {
    let name = std::ffi::CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "invalid temp entry name"))?;
    // SAFETY: directory owns a live directory fd and the final component is NUL-terminated.
    let result = unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn directory_stats(path: &Path) -> (u64, usize) {
    let Ok(entries) = fs::read_dir(path) else {
        return (0, 0);
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| fs::symlink_metadata(entry.path()).ok())
        .fold((0_u64, 0_usize), |(bytes, records), metadata| {
            (bytes.saturating_add(metadata.len()), records + 1)
        })
}

fn count_nonempty_jsonl_lines(bytes: &[u8]) -> usize {
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| line.iter().any(|byte| !byte.is_ascii_whitespace()))
        .count()
}

fn read_managed_file(data_home: &Path, path: &Path) -> io::Result<Vec<u8>> {
    if !path.starts_with(data_home) {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "managed data path is outside data home",
        ));
    }
    let mut file = open_managed_file(data_home, path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "managed data entry must be a regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(unix)]
fn open_managed_file(data_home: &Path, path: &Path) -> io::Result<File> {
    let compaction_dir = data_home.join(MANAGED_DIR);
    if path.parent() == Some(compaction_dir.as_path()) {
        let directory = open_compaction_directory(&compaction_dir)?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "invalid managed filename"))?;
        return openat_file(
            &directory,
            name,
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            0,
        );
    }
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

#[cfg(not(unix))]
fn open_managed_file(_data_home: &Path, path: &Path) -> io::Result<File> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "managed data entry must be a regular file",
        ));
    }
    OpenOptions::new().read(true).open(path)
}

#[cfg(unix)]
fn open_dedupe_temp_directory(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| {
            if matches!(error.raw_os_error(), Some(libc::ELOOP | libc::ENOTDIR)) {
                io::Error::new(
                    ErrorKind::InvalidInput,
                    "dedupe temp directory must be a real directory",
                )
            } else {
                error
            }
        })
}

#[cfg(unix)]
struct DirectoryStream(*mut libc::DIR);

#[cfg(unix)]
impl Drop for DirectoryStream {
    fn drop(&mut self) {
        // SAFETY: this stream exclusively owns the descriptor transferred by fdopendir.
        let _ = unsafe { libc::closedir(self.0) };
    }
}

#[cfg(target_vendor = "apple")]
fn clear_errno() {
    // SAFETY: __error returns the calling thread's errno slot.
    unsafe { *libc::__error() = 0 };
}

#[cfg(target_os = "linux")]
fn clear_errno() {
    // SAFETY: __errno_location returns the calling thread's errno slot.
    unsafe { *libc::__errno_location() = 0 };
}

#[cfg(any(
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
fn clear_errno() {
    // SAFETY: __error returns the calling thread's errno slot on BSD targets.
    unsafe { *libc::__error() = 0 };
}

#[cfg(all(
    unix,
    not(any(
        target_vendor = "apple",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))
))]
fn clear_errno() {}

#[cfg(unix)]
fn duplicate_directory_fd(fd: RawFd) -> io::Result<RawFd> {
    // SAFETY: dup does not take ownership of the source descriptor.
    let duplicate = unsafe { libc::dup(fd) };
    if duplicate < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(duplicate)
    }
}

#[cfg(unix)]
fn stat_temp_entry(directory: &File, name: &std::ffi::CStr) -> io::Result<Option<libc::stat>> {
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::zeroed();
    // SAFETY: `metadata` points to writable storage and `name` is NUL-terminated.
    let result = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            name.as_ptr(),
            metadata.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result != 0 {
        let error = io::Error::last_os_error();
        if error.kind() == ErrorKind::NotFound {
            return Ok(None);
        }
        return Err(error);
    }
    // SAFETY: successful fstatat initialized the complete stat structure.
    Ok(Some(unsafe { metadata.assume_init() }))
}

#[cfg(unix)]
fn purge_dedupe_temp_entries(
    directory: &File,
    entries: Vec<DedupeTempEntry>,
) -> Result<Vec<PathBuf>, JevxError> {
    let mut removed = Vec::new();
    for entry in entries {
        if !entry.is_regular_file && !entry.is_symlink {
            continue;
        }
        match unlink_dedupe_temp_entry(directory, &entry.name) {
            Ok(()) => removed.push(entry.path),
            Err(error) if error.kind() == ErrorKind::NotFound => removed.push(entry.path),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(removed)
}

#[cfg(unix)]
fn dedupe_temp_entries_from_handle(
    path: &Path,
    directory: &File,
) -> io::Result<Vec<DedupeTempEntry>> {
    // fdopendir owns the duplicate; the caller keeps `directory` for unlinkat.
    let duplicate = duplicate_directory_fd(directory.as_raw_fd())?;
    // SAFETY: fdopendir takes ownership of the successful duplicate above.
    let stream = unsafe { libc::fdopendir(duplicate) };
    if stream.is_null() {
        let error = io::Error::last_os_error();
        // SAFETY: fdopendir failed and therefore did not consume `duplicate`.
        unsafe { libc::close(duplicate) };
        return Err(error);
    }
    let stream = DirectoryStream(stream);
    let mut entries = Vec::new();
    loop {
        clear_errno();
        // SAFETY: `stream` remains live until the RAII guard is dropped.
        let entry = unsafe { libc::readdir(stream.0) };
        if entry.is_null() {
            let error = io::Error::last_os_error();
            if error.raw_os_error().is_some_and(|code| code != 0) {
                return Err(error);
            }
            break;
        }
        // SAFETY: readdir returns a live dirent whose d_name is NUL-terminated.
        let name_bytes = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if name_bytes == b"." || name_bytes == b".." {
            continue;
        }
        let name = std::ffi::CString::new(name_bytes)
            .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "invalid temp entry name"))?;
        let Some(metadata) = stat_temp_entry(directory, &name)? else {
            continue;
        };
        let entry_type = metadata.st_mode & libc::S_IFMT;
        let name = std::ffi::OsString::from_vec(name_bytes.to_vec());
        entries.push(DedupeTempEntry {
            path: path.join(&name),
            name,
            bytes: metadata.st_size.max(0) as u64,
            is_regular_file: entry_type == libc::S_IFREG,
            is_symlink: entry_type == libc::S_IFLNK,
        });
    }
    Ok(entries)
}

#[cfg(unix)]
fn dedupe_temp_entries(path: &Path) -> Result<Vec<DedupeTempEntry>, JevxError> {
    let directory = match open_dedupe_temp_directory(path) {
        Ok(directory) => directory,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    Ok(dedupe_temp_entries_from_handle(path, &directory)?)
}

#[cfg(not(unix))]
fn dedupe_temp_entries(path: &Path) -> Result<Vec<DedupeTempEntry>, JevxError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        return Err(JevxError::InvalidInput(
            "dedupe temp directory must not be a symlink".to_owned(),
        ));
    }
    if !metadata.is_dir() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        entries.push(DedupeTempEntry {
            name: entry.file_name(),
            path: entry.path(),
            bytes: metadata.len(),
            is_regular_file: metadata.is_file(),
            is_symlink: metadata.file_type().is_symlink(),
        });
    }
    Ok(entries)
}

fn dedupe_temp_paths(path: &Path) -> Result<Vec<PathBuf>, JevxError> {
    Ok(dedupe_temp_entries(path)?
        .into_iter()
        .filter(|entry| entry.is_regular_file || entry.is_symlink)
        .map(|entry| entry.path)
        .collect())
}

fn dedupe_temp_metadata(path: &Path) -> Result<Vec<Value>, JevxError> {
    Ok(dedupe_temp_entries(path)?
        .into_iter()
        .filter(|entry| entry.is_regular_file || entry.is_symlink)
        .map(|entry| json!({"path": entry.path, "bytes": entry.bytes}))
        .collect())
}

fn dedupe_temp_directory_stats(path: &Path) -> (u64, usize) {
    let Ok(entries) = dedupe_temp_entries(path) else {
        return (0, 0);
    };
    let bytes = entries
        .iter()
        .fold(0_u64, |total, entry| total.saturating_add(entry.bytes));
    (bytes, entries.len())
}

fn with_purge_locks<T, F>(data_home: &Path, operation: F) -> Result<T, JevxError>
where
    F: FnOnce(&CompactionDirectoryHandle) -> Result<T, JevxError>,
{
    #[cfg(unix)]
    {
        // Reject static path substitution before creating the dedupe lock, then validate
        // again while holding it in case the path changed during lock acquisition.
        validate_existing_compaction_paths(data_home)?;
        with_dedupe_lock(data_home, || {
            validate_existing_compaction_paths(data_home)?;
            with_compaction_lock(data_home, operation)
        })
    }
    #[cfg(not(unix))]
    {
        let _ = (data_home, operation);
        Err(JevxError::InvalidInput(
            "safe compaction purge locking is unavailable on this platform".to_owned(),
        ))
    }
}

fn with_dedupe_lock<T, F>(data_home: &Path, operation: F) -> Result<T, JevxError>
where
    F: FnOnce() -> Result<T, JevxError>,
{
    fs::create_dir_all(data_home)?;
    let lock = open_dedupe_lock(&data_home.join(DEDUPE_LOCK_FILE_NAME))?;
    lock.lock_exclusive()?;
    let result = operation();
    finish_lock(result, lock.unlock())
}

fn finish_lock<T>(result: Result<T, JevxError>, unlock: io::Result<()>) -> Result<T, JevxError> {
    match (result, unlock) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(JevxError::from(error)),
    }
}

#[cfg(unix)]
fn open_dedupe_lock(path: &Path) -> io::Result<File> {
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| {
            if error.raw_os_error() == Some(libc::ELOOP) {
                io::Error::new(
                    ErrorKind::InvalidInput,
                    "dedupe lock must be a regular file",
                )
            } else {
                error
            }
        })?;
    if !lock.metadata()?.is_file() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "dedupe lock must be a regular file",
        ));
    }
    Ok(lock)
}

#[cfg(not(unix))]
fn open_dedupe_lock(path: &Path) -> io::Result<File> {
    if fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "dedupe lock must be a regular file",
        ));
    }
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    if !lock.metadata()?.is_file() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "dedupe lock must be a regular file",
        ));
    }
    Ok(lock)
}

fn with_compaction_lock<T, F>(data_home: &Path, operation: F) -> Result<T, JevxError>
where
    F: FnOnce(&CompactionDirectoryHandle) -> Result<T, JevxError>,
{
    let compaction_dir = data_home.join(MANAGED_DIR);
    match fs::symlink_metadata(&compaction_dir) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(JevxError::InvalidInput(
                "compaction directory must be a real directory".to_owned(),
            ));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            // create_dir (not create_dir_all) fails if another process races in a symlink.
            fs::create_dir(&compaction_dir)?;
        }
        Err(error) => return Err(error.into()),
    }
    let lock_path = compaction_dir.join(COMPACT_ASSIST_LOCK_FILE_NAME);
    match fs::symlink_metadata(&lock_path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(JevxError::InvalidInput(
                "compaction lock must be a regular file".to_owned(),
            ));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let (lock, directory) = open_compaction_lock(&compaction_dir, &lock_path)?;
    lock.lock_exclusive()?;
    let result = operation(&directory);
    finish_lock(result, lock.unlock())
}

#[cfg(unix)]
fn open_compaction_lock(
    compaction_dir: &Path,
    _lock_path: &Path,
) -> Result<(File, CompactionDirectoryHandle), JevxError> {
    let directory = open_compaction_directory(compaction_dir)?;
    let lock = open_compaction_lock_at(&directory)?;
    Ok((lock, directory))
}

#[cfg(unix)]
fn open_compaction_directory(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

#[cfg(unix)]
fn open_compaction_lock_at(directory: &File) -> io::Result<File> {
    openat_file(
        directory,
        COMPACT_ASSIST_LOCK_FILE_NAME,
        libc::O_CREAT | libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        0o600,
    )
    .map_err(|error| {
        if error.raw_os_error() == Some(libc::ELOOP) {
            io::Error::new(
                ErrorKind::InvalidInput,
                "compaction lock must be a regular file",
            )
        } else {
            error
        }
    })
}

#[cfg(unix)]
fn openat_file(directory: &File, name: &str, flags: i32, mode: libc::mode_t) -> io::Result<File> {
    let name = std::ffi::CString::new(name)
        .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "invalid managed filename"))?;
    // SAFETY: `directory` owns a valid directory fd, `name` is NUL-terminated, and a
    // successful openat fd is transferred exactly once into `File` below.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            flags,
            mode as libc::c_uint,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` came from successful openat and ownership has not been transferred.
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(not(unix))]
fn open_compaction_lock(
    _compaction_dir: &Path,
    _lock_path: &Path,
) -> Result<(File, ()), JevxError> {
    Err(JevxError::InvalidInput(
        "safe compaction purge locking is unavailable on this platform".to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(home: &Path) {
        fs::create_dir_all(home.join("compaction")).expect("dir");
        fs::write(home.join("events.jsonl"), "{\"a\":1}\n{\"a\":2}\n").expect("events");
        fs::write(home.join("hooks.jsonl"), "{\"h\":1}\n\n").expect("hooks");
        fs::write(home.join("compaction/checkpoints.jsonl"), "{\"c\":1}\n").expect("cp");
        fs::write(home.join("unrelated.txt"), "keep").expect("unrelated");
    }

    #[test]
    fn inventory_lists_every_managed_file_with_sizes_and_record_counts() {
        let root = tempfile::tempdir().expect("tempdir");
        seed(root.path());
        let inventory = data_inventory(root.path()).expect("inventory");
        assert_eq!(inventory.schema_version, DATA_SCHEMA_VERSION);
        assert_eq!(inventory.data_home, root.path());
        let kinds: Vec<_> = inventory.files.iter().map(|file| file.kind).collect();
        assert_eq!(
            kinds,
            [
                "telemetry",
                "hookRecords",
                "hookDedupe",
                "hookDedupeLock",
                "hookDedupeTemps",
                "compactionLock",
                "compactionRecords",
                "compactionCheckpoints",
                "decisionReceipts",
                "reviewReceipts"
            ]
        );
        assert_eq!(inventory.files[0].records, 2);
        assert_eq!(inventory.files[1].records, 1, "blank lines are not records");
        assert!(!inventory.files[2].exists);
        assert_eq!(inventory.files[2].bytes, 0);
        assert_eq!(
            inventory.files[7].path,
            root.path().join("compaction/checkpoints.jsonl")
        );
    }

    #[test]
    fn export_returns_parsed_records_and_rejects_invalid_lines_without_echoing_them() {
        let root = tempfile::tempdir().expect("tempdir");
        seed(root.path());
        let exported =
            export_data(&data_inventory(root.path()).expect("inventory")).expect("export");
        assert_eq!(exported["schemaVersion"], DATA_SCHEMA_VERSION);
        assert_eq!(exported["records"]["telemetry"], json!([{"a":1},{"a":2}]));
        assert_eq!(exported["records"]["compactionRecords"], json!([]));

        fs::write(root.path().join("hooks.jsonl"), "not-json secret-token\n").expect("bad");
        let error = export_data(&data_inventory(root.path()).expect("inventory"))
            .expect_err("invalid line");
        let message = error.to_string();
        assert!(message.contains("hooks.jsonl:1"));
        assert!(!message.contains("secret-token"));
    }

    #[cfg(unix)]
    #[test]
    fn inventory_and_export_do_not_read_through_managed_file_symlinks() {
        let root = tempfile::tempdir().expect("home");
        let elsewhere = tempfile::tempdir().expect("outside home");
        seed(root.path());
        let external_file = elsewhere.path().join("external.jsonl");
        fs::write(&external_file, "{\"secret\":\"external-only\"}\n").expect("external data");
        fs::remove_file(root.path().join("hooks.jsonl")).expect("remove managed file");
        std::os::unix::fs::symlink(&external_file, root.path().join("hooks.jsonl"))
            .expect("managed symlink");

        let inventory = data_inventory(root.path()).expect("inventory");
        let hook_file = inventory
            .files
            .iter()
            .find(|file| file.kind == "hookRecords")
            .expect("hook inventory");
        assert_eq!(
            hook_file.records, 0,
            "inventory must not read a link target"
        );
        let error = export_data(&inventory).expect_err("export must reject a managed symlink");
        assert!(!error.to_string().contains("external-only"));
        assert!(
            external_file.exists(),
            "export never mutates the link target"
        );
    }

    #[cfg(unix)]
    #[test]
    fn managed_file_reader_rejects_external_and_non_file_paths() {
        let root = tempfile::tempdir().expect("home");
        let elsewhere = tempfile::tempdir().expect("outside home");
        let external_file = elsewhere.path().join("external.jsonl");
        fs::write(&external_file, "{}\n").expect("external file");
        assert_eq!(
            read_managed_file(root.path(), &external_file)
                .expect_err("external path must be rejected")
                .kind(),
            ErrorKind::InvalidInput
        );

        let directory = root.path().join("managed-directory");
        fs::create_dir(&directory).expect("managed directory");
        assert_eq!(
            read_managed_file(root.path(), &directory)
                .expect_err("directories are not managed data files")
                .kind(),
            ErrorKind::InvalidInput
        );
    }

    #[cfg(unix)]
    #[test]
    fn data_export_rejects_a_symlinked_dedupe_temp_directory() {
        let root = tempfile::tempdir().expect("home");
        let elsewhere = tempfile::tempdir().expect("outside home");
        let external_file = elsewhere.path().join("external.tmp");
        fs::write(&external_file, "external-only").expect("external temp");
        std::os::unix::fs::symlink(elsewhere.path(), root.path().join(DEDUPE_TEMP_DIR_NAME))
            .expect("temp directory symlink");
        let inventory = data_inventory(root.path()).expect("inventory");

        let error = export_data(&inventory).expect_err("export must reject temp directory symlink");

        assert!(!error.to_string().contains("external-only"));
        assert!(external_file.exists());
    }

    #[test]
    fn dedupe_temp_path_rejects_non_directory_and_skips_missing_entries() {
        let root = tempfile::tempdir().expect("home");
        let non_directory = root.path().join("not-a-directory");
        fs::write(&non_directory, "not temp metadata").expect("file entry");
        assert!(dedupe_temp_paths(&non_directory).is_err());
        assert!(
            dedupe_temp_paths(&root.path().join("missing"))
                .expect("missing temp directory")
                .is_empty()
        );
        assert!(dedupe_temp_paths(Path::new("\0")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn unix_temp_fd_helpers_handle_missing_entries_and_skip_directories() {
        let root = tempfile::tempdir().expect("home");
        let temp_dir = root.path().join(DEDUPE_TEMP_DIR_NAME);
        fs::create_dir(&temp_dir).expect("temp directory");
        let directory = File::open(&temp_dir).expect("directory handle");
        let regular = temp_dir.join("regular.tmp");
        fs::write(&regular, "data").expect("regular entry");
        let nested = temp_dir.join("nested");
        fs::create_dir(&nested).expect("nested directory");

        assert!(duplicate_directory_fd(-1).is_err());
        let regular_name = std::ffi::CString::new("regular.tmp").expect("name");
        let missing_name = std::ffi::CString::new("missing.tmp").expect("name");
        assert!(
            stat_temp_entry(&directory, &regular_name)
                .expect("stat regular entry")
                .is_some()
        );
        assert!(
            stat_temp_entry(&directory, &missing_name)
                .expect("stat missing entry")
                .is_none()
        );
        let regular_file_handle = File::open(&regular).expect("regular file handle");
        assert_eq!(
            stat_temp_entry(&regular_file_handle, &regular_name)
                .expect_err("fstatat requires a directory descriptor")
                .raw_os_error(),
            Some(libc::ENOTDIR)
        );
        let device_handle = File::open("/dev/null").expect("character device");
        assert!(dedupe_temp_entries_from_handle(&temp_dir, &device_handle).is_err());
        assert_eq!(
            unlink_dedupe_temp_entry(&directory, std::ffi::OsStr::new("missing.tmp"))
                .expect_err("missing entry remains a race-safe not-found")
                .kind(),
            ErrorKind::NotFound
        );
        assert_eq!(
            unlink_dedupe_temp_entry(&directory, std::ffi::OsStr::new("\0invalid"))
                .expect_err("NUL is rejected before unlinkat")
                .kind(),
            ErrorKind::InvalidInput
        );

        let removed = purge_dedupe_temp_entries(
            &directory,
            vec![
                DedupeTempEntry {
                    name: "regular.tmp".into(),
                    path: regular.clone(),
                    bytes: 4,
                    is_regular_file: true,
                    is_symlink: false,
                },
                DedupeTempEntry {
                    name: "nested".into(),
                    path: nested.clone(),
                    bytes: 0,
                    is_regular_file: false,
                    is_symlink: false,
                },
                DedupeTempEntry {
                    name: "raced.tmp".into(),
                    path: temp_dir.join("raced.tmp"),
                    bytes: 0,
                    is_regular_file: true,
                    is_symlink: false,
                },
            ],
        )
        .expect("unlink entries relative to held directory");

        assert!(removed.contains(&regular));
        assert!(removed.contains(&temp_dir.join("raced.tmp")));
        assert!(!regular.exists());
        assert!(nested.is_dir(), "purge must skip nested directories");
        let error = purge_dedupe_temp_entries(
            &directory,
            vec![DedupeTempEntry {
                name: "nested".into(),
                path: nested.clone(),
                bytes: 0,
                is_regular_file: true,
                is_symlink: false,
            }],
        )
        .expect_err("a regular file replaced by a directory is an error");
        assert!(error.to_string().starts_with("I/O error:"));
        let not_a_directory = temp_dir.join("not-a-directory");
        fs::write(&not_a_directory, "file").expect("stats error fixture");
        assert_eq!(dedupe_temp_directory_stats(&not_a_directory), (0, 0));
    }

    #[cfg(unix)]
    #[test]
    fn empty_temp_cleanup_is_idempotent_and_checks_open_directory_identity() {
        let root = tempfile::tempdir().expect("home");
        cleanup_empty_dedupe_temp_directory(root.path()).expect("missing temp directory");

        let invalid_home = root.path().join("invalid-home");
        fs::create_dir(&invalid_home).expect("invalid home");
        fs::write(invalid_home.join(DEDUPE_TEMP_DIR_NAME), "not a directory")
            .expect("regular file at directory path");
        assert!(cleanup_empty_dedupe_temp_directory(&invalid_home).is_err());

        let temp_dir = root.path().join(DEDUPE_TEMP_DIR_NAME);
        fs::create_dir(&temp_dir).expect("temp directory");
        fs::write(temp_dir.join("pending.tmp"), "pending").expect("pending entry");
        cleanup_empty_dedupe_temp_directory(root.path()).expect("nonempty temp directory");
        assert!(temp_dir.join("pending.tmp").exists());

        let empty_dir = root.path().join("renamed-temp");
        fs::create_dir(&empty_dir).expect("empty directory");
        let directory = open_dedupe_temp_directory(&empty_dir).expect("open empty directory");
        let replaced_path = root.path().join("moved-temp");
        fs::rename(&empty_dir, &replaced_path).expect("rename directory while handle is open");
        assert!(
            !ensure_dedupe_temp_directory_identity(&empty_dir, &directory)
                .expect("missing path is not the opened directory")
        );
        fs::create_dir(&empty_dir).expect("replacement directory");
        assert!(
            ensure_dedupe_temp_directory_identity(&empty_dir, &directory)
                .expect_err("replacement path must not match the open directory")
                .to_string()
                .contains("changed")
        );

        remove_empty_directory(&empty_dir).expect("missing directory is idempotent");
        remove_empty_directory(&temp_dir).expect("nonempty directory is left intact");
        assert!(temp_dir.is_dir());
        assert!(replaced_path.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn dedupe_lock_rejects_fifo_and_lock_finish_preserves_primary_errors() {
        use std::os::unix::ffi::OsStrExt;

        let root = tempfile::tempdir().expect("home");
        let fifo = root.path().join("hook-dedupe.lock");
        let fifo_c = std::ffi::CString::new(fifo.as_os_str().as_bytes()).expect("path");
        // SAFETY: fifo_c is a valid NUL-terminated path and mkfifo does not retain it.
        let result = unsafe { libc::mkfifo(fifo_c.as_ptr(), 0o600) };
        assert_eq!(result, 0, "create FIFO fixture");
        assert_eq!(
            open_dedupe_lock(&fifo)
                .expect_err("lock file must be regular")
                .kind(),
            ErrorKind::InvalidInput
        );

        let external_lock = root.path().join("external-lock");
        fs::write(&external_lock, "external").expect("external lock target");
        let linked_lock = root.path().join("linked-lock");
        std::os::unix::fs::symlink(&external_lock, &linked_lock).expect("lock symlink");
        assert_eq!(
            open_dedupe_lock(&linked_lock)
                .expect_err("lock symlinks must not be followed")
                .kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            fs::read_to_string(external_lock).expect("target unchanged"),
            "external"
        );
        let non_directory_parent = root.path().join("not-a-directory");
        fs::write(&non_directory_parent, "file").expect("non-directory parent");
        assert_eq!(
            open_dedupe_lock(&non_directory_parent.join("lock"))
                .expect_err("ordinary open errors are preserved")
                .kind(),
            ErrorKind::NotADirectory
        );

        let operation_error = finish_lock::<()>(
            Err(JevxError::InvalidInput("operation failed".to_owned())),
            Err(io::Error::other("unlock failed")),
        )
        .expect_err("operation error takes precedence");
        assert!(operation_error.to_string().contains("operation failed"));
        assert!(finish_lock(Ok(7), Ok(())).expect("successful unlock") == 7);
        assert!(
            finish_lock::<()>(Ok(()), Err(io::Error::other("unlock failed")))
                .expect_err("unlock failure is reported")
                .to_string()
                .contains("unlock failed")
        );
    }

    #[test]
    fn inventory_counts_children_of_a_managed_directory_without_opening_links() {
        let root = tempfile::tempdir().expect("home");
        let managed_directory = root.path().join("events.jsonl");
        fs::create_dir(&managed_directory).expect("managed directory");
        fs::write(managed_directory.join("entry"), "1234").expect("entry");

        let inventory = data_inventory(root.path()).expect("inventory");
        let telemetry = inventory
            .files
            .iter()
            .find(|file| file.kind == "telemetry")
            .expect("telemetry inventory");

        assert_eq!(telemetry.bytes, 4);
        assert_eq!(telemetry.records, 1);
    }

    #[test]
    fn purge_temp_inventory_ignores_a_directory_removed_after_snapshot() {
        let root = tempfile::tempdir().expect("home");
        let temp_dir = root.path().join(DEDUPE_TEMP_DIR_NAME);
        fs::create_dir(&temp_dir).expect("temp dir");
        fs::write(temp_dir.join("crashed.tmp"), "partial").expect("temp state");
        let inventory = data_inventory(root.path()).expect("inventory before removal");
        fs::remove_dir_all(&temp_dir).expect("remove stale temp directory");

        assert!(
            purge_targets(&inventory)
                .expect("stale temp directory is skipped")
                .is_empty()
        );
    }

    #[cfg(unix)]
    #[test]
    fn confirmed_purge_unlinks_a_dedupe_temp_directory_symlink_only() {
        let root = tempfile::tempdir().expect("home");
        let elsewhere = tempfile::tempdir().expect("outside home");
        let external_file = elsewhere.path().join("external.tmp");
        fs::write(&external_file, "external-only").expect("external temp");
        let temp_dir = root.path().join(DEDUPE_TEMP_DIR_NAME);
        std::os::unix::fs::symlink(elsewhere.path(), &temp_dir).expect("temp directory symlink");
        let inventory = data_inventory(root.path()).expect("inventory");

        let report = purge_data(&inventory, true).expect("purge managed symlink");

        assert!(report.removed.contains(&temp_dir));
        assert!(fs::symlink_metadata(&temp_dir).is_err());
        assert!(
            external_file.exists(),
            "purge must not traverse the directory link"
        );
    }

    #[test]
    fn confirmed_purge_does_not_create_data_or_lock_files_when_nothing_is_managed() {
        let root = tempfile::tempdir().expect("parent");
        let data_home = root.path().join("jevx-home");
        let inventory = data_inventory(&data_home).expect("empty inventory");

        let report = purge_data(&inventory, true).expect("empty purge");

        assert!(!report.dry_run);
        assert!(report.removed.is_empty());
        assert!(
            !data_home.exists(),
            "empty purge must not create persistent locks"
        );
    }

    #[test]
    fn confirmed_purge_without_targets_does_not_create_missing_stable_locks() {
        let root = tempfile::tempdir().expect("parent");
        let data_home = root.path().join("jevx-home");
        let compaction_dir = data_home.join(MANAGED_DIR);
        fs::create_dir_all(&compaction_dir).expect("empty compaction directory");

        purge_data(&data_inventory(&data_home).expect("empty inventory"), true)
            .expect("purge empty compaction directory");
        assert!(
            !compaction_dir.join(COMPACT_ASSIST_LOCK_FILE_NAME).exists(),
            "an existing empty directory must not gain a lock file"
        );

        let compact_lock = compaction_dir.join(COMPACT_ASSIST_LOCK_FILE_NAME);
        fs::write(&compact_lock, "existing lock").expect("existing stable lock");
        purge_data(
            &data_inventory(&data_home).expect("lock-only inventory"),
            true,
        )
        .expect("purge lock-only inventory");
        assert_eq!(
            fs::read_to_string(compact_lock).expect("stable lock remains"),
            "existing lock"
        );
        assert!(
            !data_home.join(DEDUPE_LOCK_FILE_NAME).exists(),
            "a no-op purge must not create the other stable lock"
        );
    }

    #[test]
    fn no_op_purge_preserves_an_empty_dedupe_temp_directory_without_a_lock() {
        let root = tempfile::tempdir().expect("home");
        let temp_dir = root.path().join(DEDUPE_TEMP_DIR_NAME);
        fs::create_dir(&temp_dir).expect("empty temp directory");

        purge_data(&data_inventory(root.path()).expect("inventory"), true)
            .expect("purge empty temp directory");

        assert!(temp_dir.is_dir(), "no-op purge must not race Hook writes");
        assert!(!root.path().join(DEDUPE_LOCK_FILE_NAME).exists());
    }

    #[test]
    fn confirmed_purge_cleans_empty_dedupe_temp_directory_while_holding_locks() {
        let root = tempfile::tempdir().expect("home");
        let temp_dir = root.path().join(DEDUPE_TEMP_DIR_NAME);
        fs::create_dir(&temp_dir).expect("empty temp directory");
        fs::write(root.path().join("events.jsonl"), "event\n").expect("managed file");

        let report = purge_data(&data_inventory(root.path()).expect("inventory"), true)
            .expect("purge managed data");

        assert!(!temp_dir.exists());
        assert!(report.removed.contains(&root.path().join("events.jsonl")));
        assert!(root.path().join(DEDUPE_LOCK_FILE_NAME).exists());
    }

    #[cfg(unix)]
    #[test]
    fn compaction_relative_purge_rejects_invalid_and_missing_names() {
        let root = tempfile::tempdir().expect("home");
        let compaction_dir = root.path().join(MANAGED_DIR);
        fs::create_dir(&compaction_dir).expect("compaction dir");
        let directory = open_compaction_directory(&compaction_dir).expect("directory handle");
        let inventory = data_inventory(root.path()).expect("inventory");

        let invalid_name = compaction_dir.join("..");
        assert_eq!(
            remove_managed_file(&inventory, &invalid_name, &directory)
                .expect_err("dot-dot is not a managed leaf name")
                .kind(),
            ErrorKind::InvalidInput
        );
        let missing_name = compaction_dir.join("missing.jsonl");
        assert_eq!(
            remove_managed_file(&inventory, &missing_name, &directory)
                .expect_err("missing entry must remain an explicit error")
                .kind(),
            ErrorKind::NotFound
        );
    }

    #[cfg(unix)]
    #[test]
    fn compaction_lock_open_preserves_non_symlink_open_errors() {
        let root = tempfile::tempdir().expect("home");
        let compaction_dir = root.path().join(MANAGED_DIR);
        fs::create_dir(&compaction_dir).expect("compaction dir");
        fs::create_dir(compaction_dir.join(COMPACT_ASSIST_LOCK_FILE_NAME))
            .expect("lock path directory");
        let directory = open_compaction_directory(&compaction_dir).expect("directory handle");

        let error = open_compaction_lock_at(&directory).expect_err("directory is not a lock file");

        assert_ne!(error.raw_os_error(), Some(libc::ELOOP));
        assert!(!error.to_string().contains("must be a regular file"));
    }

    #[test]
    fn purge_is_a_dry_run_until_confirmed_and_only_removes_managed_files() {
        let root = tempfile::tempdir().expect("tempdir");
        seed(root.path());
        let inventory = data_inventory(root.path()).expect("inventory");

        let preview = purge_data(&inventory, false).expect("preview");
        assert!(preview.dry_run);
        assert_eq!(preview.removed.len(), 3);
        assert!(root.path().join("events.jsonl").exists());
        fs::remove_file(root.path().join("events.jsonl")).expect("simulate concurrent removal");

        let report = purge_data(&inventory, true).expect("purge");
        assert!(!report.dry_run);
        assert_eq!(report.removed.len(), 2);
        assert!(!report.removed.contains(&root.path().join("events.jsonl")));
        assert!(!root.path().join("events.jsonl").exists());
        assert!(root.path().join("compaction").exists());
        assert!(
            root.path().join("compaction/.compact-assist.lock").exists(),
            "stable compact lock is retained"
        );
        assert!(root.path().join("unrelated.txt").exists());

        assert_eq!(directory_stats(&root.path().join("missing")), (0, 0));
        let error = with_dedupe_lock(root.path(), || -> Result<(), JevxError> {
            Err(JevxError::InvalidInput("operation failed".to_owned()))
        })
        .expect_err("lock operation error");
        assert!(error.to_string().contains("operation failed"));

        let again =
            purge_data(&data_inventory(root.path()).expect("inventory"), true).expect("again");
        assert!(again.removed.is_empty());
    }

    #[test]
    fn confirmed_purge_reenumerates_temp_files_after_inventory_snapshot() {
        let root = tempfile::tempdir().expect("tempdir");
        let inventory = data_inventory(root.path()).expect("initial inventory");
        let temp_dir = root.path().join(DEDUPE_TEMP_DIR_NAME);
        fs::create_dir_all(&temp_dir).expect("temp directory");
        let temp_path = temp_dir.join(".hook-dedupe.json.crashed.tmp");
        fs::write(&temp_path, "partial").expect("temp state");

        let report = purge_data(&inventory, true).expect("purge current inventory");

        assert!(report.removed.contains(&temp_path));
        assert!(!temp_path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn purge_aborts_if_compaction_directory_changes_after_lock_acquisition() {
        let root = tempfile::tempdir().expect("home");
        seed(root.path());
        let compaction_dir = root.path().join(MANAGED_DIR);
        let moved_dir = root.path().join("compaction-locked");

        let result = with_purge_locks(root.path(), |locked_directory| {
            fs::rename(&compaction_dir, &moved_dir)?;
            fs::create_dir(&compaction_dir)?;
            purge_current_inventory(root.path(), locked_directory)
        });

        assert!(
            result.is_err(),
            "purge must not act on a replacement directory"
        );
        assert!(root.path().join("events.jsonl").exists());
        assert!(moved_dir.join("checkpoints.jsonl").exists());
    }

    #[test]
    fn confirmed_purge_reports_managed_path_removal_errors() {
        let root = tempfile::tempdir().expect("tempdir");
        fs::create_dir(root.path().join("events.jsonl")).expect("unexpected managed directory");
        let inventory = data_inventory(root.path()).expect("inventory");
        let error = purge_data(&inventory, true).expect_err("directory cannot be removed as file");
        assert!(matches!(error, JevxError::Io(_)));
    }

    #[test]
    fn purge_keeps_compaction_dir_with_foreign_files() {
        let root = tempfile::tempdir().expect("tempdir");
        seed(root.path());
        fs::write(root.path().join("compaction/notes.md"), "mine").expect("foreign");
        purge_data(&data_inventory(root.path()).expect("inventory"), true).expect("purge");
        assert!(root.path().join("compaction/notes.md").exists());
    }

    #[test]
    fn unreadable_or_non_utf8_data_can_still_be_listed_and_purged() {
        let root = tempfile::tempdir().expect("tempdir");
        fs::write(root.path().join("events.jsonl"), b"\xff\xfe{}\n{\"a\":1}\n").expect("binary");
        let inventory = data_inventory(root.path()).expect("inventory");
        assert!(inventory.files[0].exists);
        assert_eq!(inventory.files[0].records, 2);
        let error = export_data(&inventory).expect_err("invalid line");
        assert!(error.to_string().contains("events.jsonl:1"));
        let report = purge_data(&inventory, true).expect("purge");
        assert_eq!(report.removed.len(), 1);
        assert!(!root.path().join("events.jsonl").exists());
    }

    #[cfg(unix)]
    #[test]
    fn purge_rejects_a_symlinked_compaction_directory_without_writing_through_it() {
        let root = tempfile::tempdir().expect("tempdir");
        let elsewhere = tempfile::tempdir().expect("elsewhere");
        fs::write(root.path().join("events.jsonl"), "{}\n").expect("managed event");
        fs::write(elsewhere.path().join("checkpoints.jsonl"), "{}\n").expect("cp");
        std::os::unix::fs::symlink(elsewhere.path(), root.path().join("compaction")).expect("link");
        let inventory = data_inventory(root.path()).expect("inventory");
        assert!(
            !inventory
                .files
                .iter()
                .find(|file| file.kind == "compactionCheckpoints")
                .expect("checkpoint inventory")
                .exists,
            "inventory must not expose data reached through a symlinked parent"
        );
        let error =
            purge_data(&inventory, true).expect_err("purge must not follow compaction symlink");
        assert!(error.to_string().contains("compaction directory"));
        assert!(
            root.path().join("compaction").exists(),
            "the link itself is kept"
        );
        assert!(elsewhere.path().exists());
        assert!(root.path().join("events.jsonl").exists());
        assert!(
            elsewhere.path().join("checkpoints.jsonl").exists(),
            "managed data outside JEVX_HOME is untouched"
        );
        assert!(
            !elsewhere
                .path()
                .join(COMPACT_ASSIST_LOCK_FILE_NAME)
                .exists(),
            "purge must not create a lock outside JEVX_HOME"
        );
        assert!(
            !root.path().join(DEDUPE_LOCK_FILE_NAME).exists(),
            "invalid compaction path must be rejected before creating the dedupe lock"
        );
    }

    #[cfg(unix)]
    #[test]
    fn purge_rejects_a_symlinked_compaction_lock_without_writing_through_it() {
        let root = tempfile::tempdir().expect("tempdir");
        let elsewhere = tempfile::tempdir().expect("elsewhere");
        fs::write(root.path().join("events.jsonl"), "{}\n").expect("managed event");
        fs::create_dir(root.path().join("compaction")).expect("compaction dir");
        let external_lock = elsewhere.path().join("external.lock");
        fs::write(&external_lock, "keep").expect("external lock");
        std::os::unix::fs::symlink(
            &external_lock,
            root.path()
                .join("compaction")
                .join(COMPACT_ASSIST_LOCK_FILE_NAME),
        )
        .expect("symlink lock");

        let error = purge_data(&data_inventory(root.path()).expect("inventory"), true)
            .expect_err("purge must not follow lock symlink");

        assert!(error.to_string().contains("compaction lock"));
        assert_eq!(
            fs::read_to_string(external_lock).expect("external lock content"),
            "keep"
        );
        assert!(root.path().join("events.jsonl").exists());
        assert!(
            !root.path().join(DEDUPE_LOCK_FILE_NAME).exists(),
            "invalid compaction lock must be rejected before creating the dedupe lock"
        );
    }

    #[cfg(unix)]
    #[test]
    fn purge_rejects_a_symlinked_dedupe_lock_without_touching_external_data() {
        let root = tempfile::tempdir().expect("home");
        let elsewhere = tempfile::tempdir().expect("outside home");
        fs::write(root.path().join("events.jsonl"), "{}\n").expect("managed event");
        let external_lock = elsewhere.path().join("external.lock");
        fs::write(&external_lock, "external lock content").expect("external lock");
        std::os::unix::fs::symlink(&external_lock, root.path().join(DEDUPE_LOCK_FILE_NAME))
            .expect("dedupe lock symlink");

        let error = purge_data(&data_inventory(root.path()).expect("inventory"), true)
            .expect_err("purge must not follow dedupe lock symlink");

        assert!(error.to_string().contains("dedupe lock"));
        assert!(root.path().join("events.jsonl").exists());
        assert_eq!(
            fs::read_to_string(external_lock).expect("external lock content"),
            "external lock content"
        );
    }

    #[cfg(unix)]
    #[test]
    fn temp_child_path_deletion_must_not_follow_a_replaced_parent_symlink() {
        let root = tempfile::tempdir().expect("home");
        let elsewhere = tempfile::tempdir().expect("outside home");
        let temp_dir = root.path().join(DEDUPE_TEMP_DIR_NAME);
        fs::create_dir(&temp_dir).expect("temp directory");
        fs::write(temp_dir.join("x"), "managed temp").expect("managed temp file");
        fs::write(elsewhere.path().join("x"), "external file").expect("external file");
        let compaction_dir = root.path().join(MANAGED_DIR);
        fs::create_dir(&compaction_dir).expect("compaction directory");
        let locked_directory = open_compaction_directory(&compaction_dir).expect("lock directory");
        let inventory = data_inventory(root.path()).expect("inventory");
        let moved_dir = root.path().join("dedupe-temp-original");
        fs::rename(&temp_dir, &moved_dir).expect("move original temp directory");
        std::os::unix::fs::symlink(elsewhere.path(), &temp_dir).expect("replace with symlink");

        let error = remove_managed_file(&inventory, &temp_dir.join("x"), &locked_directory)
            .expect_err("temp children must be deleted relative to their held directory fd");

        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        assert!(elsewhere.path().join("x").exists());
        assert!(moved_dir.join("x").exists());
    }

    #[cfg(unix)]
    #[test]
    fn temp_directory_fd_keeps_listing_and_unlink_anchored_after_path_swap() {
        let root = tempfile::tempdir().expect("home");
        let elsewhere = tempfile::tempdir().expect("outside home");
        let temp_dir = root.path().join(DEDUPE_TEMP_DIR_NAME);
        fs::create_dir(&temp_dir).expect("temp directory");
        fs::write(temp_dir.join("x"), "managed temp").expect("managed temp file");
        fs::write(elsewhere.path().join("x"), "external file").expect("external file");
        let directory = open_dedupe_temp_directory(&temp_dir).expect("verified temp directory");
        let moved_dir = root.path().join("dedupe-temp-original");
        fs::rename(&temp_dir, &moved_dir).expect("move original temp directory");
        std::os::unix::fs::symlink(elsewhere.path(), &temp_dir).expect("replace with symlink");

        let entries = dedupe_temp_entries_from_handle(&temp_dir, &directory)
            .expect("enumerate the held directory");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "x");
        unlink_dedupe_temp_entry(&directory, &entries[0].name)
            .expect("unlink relative to the held directory");

        assert!(!moved_dir.join("x").exists());
        assert!(elsewhere.path().join("x").exists());
    }

    #[cfg(unix)]
    #[test]
    fn compaction_lock_creation_uses_the_verified_directory_handle() {
        let root = tempfile::tempdir().expect("tempdir");
        let external = tempfile::tempdir().expect("external");
        let compaction_dir = root.path().join("compaction");
        let moved_dir = root.path().join("compaction-original");
        fs::create_dir(&compaction_dir).expect("compaction directory");
        let directory =
            open_compaction_directory(&compaction_dir).expect("verified directory handle");

        fs::rename(&compaction_dir, &moved_dir).expect("move verified directory");
        std::os::unix::fs::symlink(external.path(), &compaction_dir).expect("replace with symlink");
        drop(open_compaction_lock_at(&directory).expect("lock relative to verified directory"));

        assert!(moved_dir.join(COMPACT_ASSIST_LOCK_FILE_NAME).exists());
        assert!(!external.path().join(COMPACT_ASSIST_LOCK_FILE_NAME).exists());
    }

    #[cfg(unix)]
    #[test]
    fn compaction_lock_open_rejects_directory_and_lock_symlinks() {
        let root = tempfile::tempdir().expect("tempdir");
        let external = tempfile::tempdir().expect("external");
        let directory_link = root.path().join("compaction-link");
        std::os::unix::fs::symlink(external.path(), &directory_link).expect("directory symlink");
        assert!(open_compaction_directory(&directory_link).is_err());

        let directory = open_compaction_directory(external.path()).expect("external directory");
        let missing_target = root.path().join("must-not-be-created.lock");
        std::os::unix::fs::symlink(
            &missing_target,
            external.path().join(COMPACT_ASSIST_LOCK_FILE_NAME),
        )
        .expect("dangling lock symlink");
        let error = open_compaction_lock_at(&directory).expect_err("lock symlink refused");
        assert!(error.to_string().contains("compaction lock"));
        assert!(!missing_target.exists());
    }

    #[test]
    fn decision_receipts_are_listed_exported_and_purged() {
        let root = tempfile::tempdir().expect("tempdir");
        fs::write(
            root.path().join("decisions.jsonl"),
            "{\"decision\":\"none\"}\n",
        )
        .expect("receipts");
        let inventory = data_inventory(root.path()).expect("inventory");
        let receipts = inventory
            .files
            .iter()
            .find(|file| file.kind == "decisionReceipts")
            .expect("decision receipts are managed");
        assert_eq!(receipts.records, 1);
        let exported = export_data(&inventory).expect("export");
        assert_eq!(
            exported["records"]["decisionReceipts"][0]["decision"],
            "none"
        );
        purge_data(&inventory, true).expect("purge");
        assert!(!root.path().join("decisions.jsonl").exists());
    }

    #[test]
    fn hook_dedupe_state_is_listed_exported_and_purged() {
        let root = tempfile::tempdir().expect("tempdir");
        fs::write(
            root.path().join("hook-dedupe.json"),
            "{\"schemaVersion\":1,\"entries\":[]}\n",
        )
        .expect("dedupe state");
        let inventory = data_inventory(root.path()).expect("inventory");
        let dedupe = inventory
            .files
            .iter()
            .find(|file| file.kind == "hookDedupe")
            .expect("dedupe state is managed");
        assert!(dedupe.exists);
        let exported = export_data(&inventory).expect("export");
        assert_eq!(exported["records"]["hookDedupe"][0]["schemaVersion"], 1);
        let preview = purge_data(&inventory, false).expect("preview");
        assert!(
            preview
                .removed
                .iter()
                .any(|path| path.ends_with("hook-dedupe.json"))
        );
        purge_data(&inventory, true).expect("purge");
        assert!(!root.path().join("hook-dedupe.json").exists());
    }

    #[test]
    fn orphaned_dedupe_temps_are_exported_and_purged_without_removing_lock() {
        let root = tempfile::tempdir().expect("tempdir");
        let temp_dir = root.path().join(".hook-dedupe-tmp");
        fs::create_dir_all(&temp_dir).expect("temp dir");
        let temp_path = temp_dir.join(".hook-dedupe.json.1.0.tmp");
        fs::write(&temp_path, "partial state").expect("temp state");
        fs::write(root.path().join("hook-dedupe.lock"), "").expect("lock");

        let inventory = data_inventory(root.path()).expect("inventory");
        let temps = inventory
            .files
            .iter()
            .find(|file| file.kind == "hookDedupeTemps")
            .expect("temp inventory");
        assert!(temps.exists);
        assert_eq!(temps.records, 1);
        let exported = export_data(&inventory).expect("export");
        assert_eq!(exported["records"]["hookDedupeTemps"][0]["bytes"], 13);

        let preview = purge_data(&inventory, false).expect("preview");
        assert!(preview.removed.iter().any(|path| path == &temp_path));
        purge_data(&inventory, true).expect("purge");
        assert!(!temp_path.exists());
        assert!(!temp_dir.exists());
        assert!(root.path().join("hook-dedupe.lock").exists());
    }
}
