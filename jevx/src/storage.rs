use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use fs2::FileExt;
use serde::Serialize;

use crate::JevxError;

pub(crate) fn append_json_line<T: Serialize>(path: &Path, value: &T) -> Result<(), JevxError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut encoded = serde_json::to_vec(value)?;
    encoded.push(b'\n');
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.lock_exclusive()?;
    let write_result = file.write_all(&encoded).map_err(JevxError::from);
    let unlock_result = file.unlock().map_err(JevxError::from);
    write_result?;
    unlock_result?;
    Ok(())
}

pub(crate) fn open_or_create_managed_directory(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "managed directory must not be a symlink or special file",
            ));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => match fs::create_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        },
        Err(error) => return Err(error),
    }
    open_managed_directory(path)
}

pub(crate) fn open_managed_directory(path: &Path) -> io::Result<File> {
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)?;
    verify_directory_binding(path, &directory)?;
    Ok(directory)
}

pub(crate) fn verify_directory_binding(path: &Path, directory: &File) -> io::Result<()> {
    let path_metadata = fs::symlink_metadata(path)?;
    let directory_metadata = directory.metadata()?;
    if !path_metadata.is_dir()
        || path_metadata.file_type().is_symlink()
        || path_metadata.dev() != directory_metadata.dev()
        || path_metadata.ino() != directory_metadata.ino()
    {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "managed directory changed while open",
        ));
    }
    Ok(())
}

pub(crate) fn open_managed_file_at(
    directory: &File,
    name: &str,
    flags: i32,
    mode: libc::mode_t,
) -> io::Result<File> {
    let name = safe_entry_name(name)?;
    // SAFETY: `directory` owns a directory fd and `name` is a validated NUL-terminated name.
    let descriptor = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            flags,
            mode as libc::c_uint,
        )
    };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `descriptor` is newly returned by openat and transferred exactly once.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

pub(crate) fn read_managed_file_at(
    directory: &File,
    name: &str,
) -> Result<Option<Vec<u8>>, JevxError> {
    let mut file = match open_managed_file_at(
        directory,
        name,
        libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        0,
    ) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !file.metadata()?.is_file() {
        return Err(JevxError::InvalidInput(
            "managed data entry must be a regular file".to_owned(),
        ));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

pub(crate) fn append_json_line_at<T: Serialize>(
    directory: &File,
    name: &str,
    value: &T,
) -> Result<(), JevxError> {
    let mut encoded = serde_json::to_vec(value)?;
    encoded.push(b'\n');
    let mut file = open_managed_file_at(
        directory,
        name,
        libc::O_WRONLY
            | libc::O_APPEND
            | libc::O_CREAT
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK,
        0o600,
    )?;
    if !file.metadata()?.is_file() {
        return Err(JevxError::InvalidInput(
            "managed data entry must be a regular file".to_owned(),
        ));
    }
    file.lock_exclusive()?;
    let write_result = file
        .write_all(&encoded)
        .and_then(|()| file.sync_data())
        .map_err(JevxError::from);
    let unlock_result = file.unlock().map_err(JevxError::from);
    write_result?;
    unlock_result?;
    Ok(())
}

pub(crate) fn atomic_replace_in_temp_directory(
    parent_path: &Path,
    temp_directory_name: &str,
    destination_name: &str,
    bytes: &[u8],
) -> Result<(), JevxError> {
    let parent = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(parent_path)?;
    let temp_directory = open_or_create_child_directory(&parent, temp_directory_name)?;
    for attempt in 0..100_u32 {
        let temp_name = format!(".{destination_name}.{}.{}.tmp", std::process::id(), attempt);
        let mut file = match open_managed_file_at(
            &temp_directory,
            &temp_name,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        ) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        };
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            rename_entry(&temp_directory, &temp_name, &parent, destination_name)
        })();
        if result.is_err() {
            let _ = unlink_entry(&temp_directory, &temp_name);
        }
        return result.map_err(JevxError::from);
    }
    Err(io::Error::new(
        ErrorKind::AlreadyExists,
        "could not allocate temporary managed state path",
    )
    .into())
}

fn open_or_create_child_directory(parent: &File, name: &str) -> io::Result<File> {
    let name = safe_entry_name(name)?;
    // SAFETY: `parent` owns a directory fd; name is validated and mode is private.
    let result = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) };
    if result != 0 {
        let error = io::Error::last_os_error();
        if error.kind() != ErrorKind::AlreadyExists {
            return Err(error);
        }
    }
    open_managed_file_at(
        parent,
        name.to_str().expect("validated UTF-8 name"),
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        0,
    )
}

fn safe_entry_name(name: &str) -> io::Result<std::ffi::CString> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "managed entry name must be one path component",
        ));
    }
    std::ffi::CString::new(name).map_err(|_| {
        io::Error::new(
            ErrorKind::InvalidInput,
            "managed entry name contains a NUL byte",
        )
    })
}

fn rename_entry(
    source_directory: &File,
    source_name: &str,
    destination_directory: &File,
    destination_name: &str,
) -> io::Result<()> {
    let source = safe_entry_name(source_name)?;
    let destination = safe_entry_name(destination_name)?;
    // SAFETY: both fds are live directory handles and both names are validated.
    let result = unsafe {
        libc::renameat(
            source_directory.as_raw_fd(),
            source.as_ptr(),
            destination_directory.as_raw_fd(),
            destination.as_ptr(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn unlink_entry(directory: &File, name: &str) -> io::Result<()> {
    let name = safe_entry_name(name)?;
    // SAFETY: `directory` owns a live directory fd and the name is validated.
    let result = unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;
    use tempfile::tempdir;

    #[derive(Serialize)]
    struct Record {
        value: u8,
    }

    #[test]
    fn append_json_line_creates_parent_and_keeps_records_separate() {
        let root = tempdir().expect("tempdir");
        let path = root.path().join("nested/events.jsonl");
        append_json_line(&path, &Record { value: 1 }).expect("first record");
        append_json_line(&path, &Record { value: 2 }).expect("second record");

        assert_eq!(
            fs::read_to_string(path).expect("events"),
            "{\"value\":1}\n{\"value\":2}\n"
        );
    }

    #[test]
    fn managed_directory_and_file_operations_refuse_symlinks() {
        let root = tempdir().expect("temp root");
        let outside = tempdir().expect("outside");
        let linked_directory = root.path().join("linked-directory");
        std::os::unix::fs::symlink(outside.path(), &linked_directory).expect("directory link");
        assert!(open_or_create_managed_directory(&linked_directory).is_err());

        let directory = open_or_create_managed_directory(&root.path().join("nested/managed"))
            .expect("managed directory");
        let outside_file = outside.path().join("events.jsonl");
        fs::write(&outside_file, "").expect("outside file");
        std::os::unix::fs::symlink(
            &outside_file,
            root.path().join("nested/managed/events.jsonl"),
        )
        .expect("file link");
        assert!(append_json_line_at(&directory, "events.jsonl", &Record { value: 1 }).is_err());
        assert_eq!(fs::read_to_string(outside_file).expect("outside data"), "");
        assert!(read_managed_file_at(&directory, "events.jsonl").is_err());
        assert!(
            read_managed_file_at(&directory, "missing.jsonl")
                .expect("missing managed file")
                .is_none()
        );
        assert!(open_managed_file_at(&directory, "../outside", libc::O_RDONLY, 0).is_err());
        assert!(open_managed_file_at(&directory, "bad\0name", libc::O_RDONLY, 0).is_err());

        let fifo_path = root.path().join("nested/managed/events.fifo");
        let fifo_name =
            std::ffi::CString::new(fifo_path.to_str().expect("fifo path")).expect("fifo filename");
        // SAFETY: path is NUL-terminated and no descriptor is passed to mkfifo.
        assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
        let _fifo_reader = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&fifo_path)
            .expect("nonblocking FIFO reader");
        assert!(read_managed_file_at(&directory, "events.fifo").is_err());
        assert!(append_json_line_at(&directory, "events.fifo", &Record { value: 2 }).is_err());
    }

    #[test]
    fn directory_binding_detects_replacement_and_creation_errors() {
        let root = tempdir().expect("root");
        let outside = tempdir().expect("outside");
        let regular_file = root.path().join("not-a-directory");
        fs::write(&regular_file, "file").expect("regular file");
        assert!(open_or_create_managed_directory(&regular_file).is_err());
        assert!(open_or_create_managed_directory(&regular_file.join("child")).is_err());
        assert!(open_or_create_managed_directory(&root.path().join("bad\0directory")).is_err());

        let managed_path = root.path().join("managed");
        let directory = open_or_create_managed_directory(&managed_path).expect("directory");
        let moved = root.path().join("managed-moved");
        fs::rename(&managed_path, &moved).expect("move managed dir");
        std::os::unix::fs::symlink(outside.path(), &managed_path).expect("replacement link");
        assert!(verify_directory_binding(&managed_path, &directory).is_err());
    }

    #[test]
    fn atomic_replace_uses_a_real_temp_directory_and_replaces_state() {
        let root = tempdir().expect("root");
        let outside = tempdir().expect("outside");
        atomic_replace_in_temp_directory(root.path(), ".tmp", "state.json", b"first")
            .expect("first replace");
        atomic_replace_in_temp_directory(root.path(), ".tmp", "state.json", b"second")
            .expect("second replace");
        assert_eq!(
            fs::read(root.path().join("state.json")).expect("state"),
            b"second"
        );

        let linked_root = tempdir().expect("linked root");
        std::os::unix::fs::symlink(outside.path(), linked_root.path().join(".tmp"))
            .expect("temp symlink");
        assert!(
            atomic_replace_in_temp_directory(linked_root.path(), ".tmp", "state.json", b"unsafe")
                .is_err()
        );
        assert_eq!(
            fs::read_dir(outside.path())
                .expect("outside entries")
                .count(),
            0
        );
    }

    #[test]
    fn atomic_replace_retries_collisions_and_cleans_failed_renames() {
        let root = tempdir().expect("root");
        let temp_dir = root.path().join(".tmp");
        fs::create_dir(&temp_dir).expect("temp directory");
        fs::write(
            temp_dir.join(format!(".state.json.{}.0.tmp", std::process::id())),
            "occupied",
        )
        .expect("collision");
        atomic_replace_in_temp_directory(root.path(), ".tmp", "state.json", b"written")
            .expect("retry after collision");
        assert_eq!(
            fs::read(root.path().join("state.json")).expect("state"),
            b"written"
        );

        fs::create_dir(root.path().join("directory-target")).expect("directory target");
        assert!(
            atomic_replace_in_temp_directory(
                root.path(),
                ".tmp",
                "directory-target",
                b"not a directory"
            )
            .is_err()
        );
        assert!(
            !temp_dir
                .join(format!(".directory-target.{}.0.tmp", std::process::id()))
                .exists()
        );
        assert!(
            atomic_replace_in_temp_directory(root.path(), ".tmp", &"x".repeat(250), b"too long")
                .is_err()
        );
    }

    #[test]
    fn atomic_replace_stops_after_one_hundred_collisions() {
        let root = tempdir().expect("root");
        let temp_dir = root.path().join(".tmp");
        fs::create_dir(&temp_dir).expect("temp directory");
        for attempt in 0..100_u32 {
            fs::write(
                temp_dir.join(format!(
                    ".state.json.{}.{}.tmp",
                    std::process::id(),
                    attempt
                )),
                "occupied",
            )
            .expect("collision file");
        }

        assert!(
            atomic_replace_in_temp_directory(root.path(), ".tmp", "state.json", b"no").is_err()
        );
        assert!(!root.path().join("state.json").exists());
        let directory = open_managed_directory(root.path()).expect("parent directory");
        assert!(open_or_create_child_directory(&directory, &"x".repeat(256)).is_err());
        assert!(
            unlink_entry(
                &open_managed_directory(&temp_dir).expect("temp directory"),
                "missing"
            )
            .is_err()
        );
    }
}
