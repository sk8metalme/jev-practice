use std::fs::{self, OpenOptions};
use std::io::Write;
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
}
