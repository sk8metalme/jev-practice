//! jevxが `$JEVX_HOME` に作るローカルデータの一覧・書き出し・削除。
//!
//! 対象はjevxが自分で書き込むファイルだけで、利用者が作る `.jevx/compact-context.md` や
//! Codexの `hooks.json` とそのbackupには触れない。

use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::JevxError;
use crate::hook_dedupe::{DEDUPE_LOCK_FILE_NAME, DEDUPE_TEMP_DIR_NAME};

pub const DATA_SCHEMA_VERSION: u8 = 5;

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
const MANAGED_FILES: [(&str, &str); 9] = [
    ("telemetry", "events.jsonl"),
    ("hookRecords", "hooks.jsonl"),
    ("hookDedupe", "hook-dedupe.json"),
    ("hookDedupeLock", "hook-dedupe.lock"),
    ("hookDedupeTemps", ".hook-dedupe-tmp"),
    ("compactionRecords", "compaction/hook-records.jsonl"),
    ("compactionCheckpoints", "compaction/checkpoints.jsonl"),
    ("decisionReceipts", "decisions.jsonl"),
    ("reviewReceipts", "reviews.jsonl"),
];
const MANAGED_DIR: &str = "compaction";

pub fn data_inventory(data_home: &Path) -> Result<DataInventory, JevxError> {
    let files = MANAGED_FILES
        .iter()
        .map(|(kind, relative)| {
            let path = data_home.join(relative);
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
                directory_stats(&path)
            } else {
                // 壊れた・UTF-8でないファイルでも一覧と削除ができるよう、読めなければ0件として扱う。
                (
                    metadata.len(),
                    fs::read(&path).map_or(0, |bytes| {
                        bytes
                            .split(|byte| *byte == b'\n')
                            .filter(|line| line.iter().any(|byte| !byte.is_ascii_whitespace()))
                            .count()
                    }),
                )
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
            let content = String::from_utf8_lossy(&fs::read(&file.path)?).into_owned();
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

/// `confirmed` がfalseなら削除対象を返すだけ。削除後に空になった管理ディレクトリも片付ける。
pub fn purge_data(inventory: &DataInventory, confirmed: bool) -> Result<PurgeReport, JevxError> {
    let mut removed = Vec::new();
    for file in &inventory.files {
        if !file.exists || file.kind == "hookDedupeLock" {
            continue;
        }
        if file.kind == "hookDedupeTemps" {
            removed.extend(dedupe_temp_paths(&file.path)?);
        } else {
            removed.push(file.path.clone());
        }
    }
    let purge = || -> Result<(), JevxError> {
        for path in &removed {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        let temp_dir = inventory.data_home.join(DEDUPE_TEMP_DIR_NAME);
        if fs::symlink_metadata(&temp_dir).is_ok_and(|metadata| metadata.is_dir())
            && fs::read_dir(&temp_dir)?.next().is_none()
        {
            fs::remove_dir(&temp_dir)?;
        }
        let dir = inventory.data_home.join(MANAGED_DIR);
        let is_real_dir = fs::symlink_metadata(&dir).is_ok_and(|metadata| metadata.is_dir());
        if is_real_dir && fs::read_dir(&dir)?.next().is_none() {
            fs::remove_dir(&dir)?;
        }
        Ok(())
    };
    if confirmed && needs_dedupe_lock(inventory) {
        with_dedupe_lock(&inventory.data_home, purge)?;
    } else if confirmed {
        purge()?;
    }
    Ok(PurgeReport {
        schema_version: DATA_SCHEMA_VERSION,
        dry_run: !confirmed,
        removed,
    })
}

fn directory_stats(path: &Path) -> (u64, usize) {
    let Ok(entries) = fs::read_dir(path) else {
        return (0, 0);
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.metadata().ok())
        .fold((0_u64, 0_usize), |(bytes, records), metadata| {
            (bytes.saturating_add(metadata.len()), records + 1)
        })
}

fn dedupe_temp_paths(path: &Path) -> Result<Vec<PathBuf>, JevxError> {
    if !fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir()) {
        return Ok(Vec::new());
    }
    Ok(fs::read_dir(path)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|entry| fs::symlink_metadata(entry).is_ok_and(|metadata| metadata.is_file()))
        .collect())
}

fn dedupe_temp_metadata(path: &Path) -> Result<Vec<Value>, JevxError> {
    Ok(dedupe_temp_paths(path)?
        .into_iter()
        .map(|path| {
            let bytes = fs::symlink_metadata(&path).map_or(0, |metadata| metadata.len());
            json!({"path": path, "bytes": bytes})
        })
        .collect())
}

fn needs_dedupe_lock(inventory: &DataInventory) -> bool {
    inventory.files.iter().any(|file| {
        file.exists
            && matches!(
                file.kind,
                "hookDedupe" | "hookDedupeLock" | "hookDedupeTemps"
            )
    })
}

fn with_dedupe_lock<T, F>(data_home: &Path, operation: F) -> Result<T, JevxError>
where
    F: FnOnce() -> Result<T, JevxError>,
{
    fs::create_dir_all(data_home)?;
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(data_home.join(DEDUPE_LOCK_FILE_NAME))?;
    lock.lock_exclusive()?;
    let result = operation();
    let unlock = lock.unlock();
    match (result, unlock) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(JevxError::from(error)),
    }
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
            inventory.files[6].path,
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
        assert_eq!(report.removed, preview.removed);
        assert!(!root.path().join("events.jsonl").exists());
        assert!(
            !root.path().join("compaction").exists(),
            "empty managed dir is removed"
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
    fn purge_does_not_follow_a_symlinked_managed_directory() {
        let root = tempfile::tempdir().expect("tempdir");
        let elsewhere = tempfile::tempdir().expect("elsewhere");
        fs::write(elsewhere.path().join("checkpoints.jsonl"), "{}\n").expect("cp");
        std::os::unix::fs::symlink(elsewhere.path(), root.path().join("compaction")).expect("link");
        let report = purge_data(&data_inventory(root.path()).expect("inventory"), true)
            .expect("purge succeeds");
        assert_eq!(report.removed.len(), 1);
        assert!(
            root.path().join("compaction").exists(),
            "the link itself is kept"
        );
        assert!(elsewhere.path().exists());
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
