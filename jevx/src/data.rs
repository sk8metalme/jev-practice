//! jevxが `$JEVX_HOME` に作るローカルデータの一覧・書き出し・削除。
//!
//! 対象はjevxが自分で書き込むファイルだけで、利用者が作る `.jevx/compact-context.md` や
//! Codexの `hooks.json` とそのbackupには触れない。

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::JevxError;

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
const MANAGED_FILES: [(&str, &str); 4] = [
    ("telemetry", "events.jsonl"),
    ("hookRecords", "hooks.jsonl"),
    ("compactionRecords", "compaction/hook-records.jsonl"),
    ("compactionCheckpoints", "compaction/checkpoints.jsonl"),
];
const MANAGED_DIR: &str = "compaction";

pub fn data_inventory(data_home: &Path) -> Result<DataInventory, JevxError> {
    let files = MANAGED_FILES
        .iter()
        .map(|(kind, relative)| {
            let path = data_home.join(relative);
            if !path.is_file() {
                return Ok(DataFile {
                    kind,
                    path,
                    exists: false,
                    bytes: 0,
                    records: 0,
                });
            }
            let content = fs::read_to_string(&path)?;
            Ok(DataFile {
                kind,
                exists: true,
                bytes: fs::metadata(&path)?.len(),
                records: content
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .count(),
                path,
            })
        })
        .collect::<Result<Vec<_>, JevxError>>()?;
    Ok(DataInventory {
        schema_version: 1,
        data_home: data_home.to_path_buf(),
        files,
    })
}

/// 保存済みのJSONLをkindごとの配列として返す。壊れた行は内容を出さずに位置だけ報告する。
pub fn export_data(inventory: &DataInventory) -> Result<Value, JevxError> {
    let mut records = Map::new();
    for file in &inventory.files {
        let mut values = Vec::new();
        if file.exists {
            for (index, line) in fs::read_to_string(&file.path)?.lines().enumerate() {
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
        "schemaVersion": 1,
        "dataHome": inventory.data_home,
        "records": records,
    }))
}

/// `confirmed` がfalseなら削除対象を返すだけ。削除後に空になった管理ディレクトリも片付ける。
pub fn purge_data(inventory: &DataInventory, confirmed: bool) -> Result<PurgeReport, JevxError> {
    let removed: Vec<PathBuf> = inventory
        .files
        .iter()
        .filter(|file| file.exists)
        .map(|file| file.path.clone())
        .collect();
    if confirmed {
        for path in &removed {
            fs::remove_file(path)?;
        }
        let dir = inventory.data_home.join(MANAGED_DIR);
        if dir.is_dir() && fs::read_dir(&dir)?.next().is_none() {
            fs::remove_dir(&dir)?;
        }
    }
    Ok(PurgeReport {
        schema_version: 1,
        dry_run: !confirmed,
        removed,
    })
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
        assert_eq!(inventory.schema_version, 1);
        assert_eq!(inventory.data_home, root.path());
        let kinds: Vec<_> = inventory.files.iter().map(|file| file.kind).collect();
        assert_eq!(
            kinds,
            [
                "telemetry",
                "hookRecords",
                "compactionRecords",
                "compactionCheckpoints"
            ]
        );
        assert_eq!(inventory.files[0].records, 2);
        assert_eq!(inventory.files[1].records, 1, "blank lines are not records");
        assert!(!inventory.files[2].exists);
        assert_eq!(inventory.files[2].bytes, 0);
        assert_eq!(
            inventory.files[3].path,
            root.path().join("compaction/checkpoints.jsonl")
        );
    }

    #[test]
    fn export_returns_parsed_records_and_rejects_invalid_lines_without_echoing_them() {
        let root = tempfile::tempdir().expect("tempdir");
        seed(root.path());
        let exported =
            export_data(&data_inventory(root.path()).expect("inventory")).expect("export");
        assert_eq!(exported["schemaVersion"], 1);
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

        let report = purge_data(&inventory, true).expect("purge");
        assert!(!report.dry_run);
        assert_eq!(report.removed, preview.removed);
        assert!(!root.path().join("events.jsonl").exists());
        assert!(
            !root.path().join("compaction").exists(),
            "empty managed dir is removed"
        );
        assert!(root.path().join("unrelated.txt").exists());

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
}
