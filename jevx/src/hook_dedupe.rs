use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::JevxError;
use crate::types::CandidateDecision;

pub(crate) const DEDUPE_FILE_NAME: &str = "hook-dedupe.json";
const DEDUPE_SCHEMA_VERSION: u8 = 1;
const DEDUPE_TTL_MS: u64 = 30_000;
const DEDUPE_CAPACITY: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct CachedHookDecision {
    pub(crate) key_sha256: String,
    pub(crate) created_at_ms: u64,
    pub(crate) decision: CandidateDecision,
    pub(crate) selected_skill: Option<String>,
    pub(crate) discovery_ms: Option<u64>,
    pub(crate) jev_response_ms: Option<u64>,
    pub(crate) total_ms: Option<u64>,
    pub(crate) input_tokens: Option<u64>,
    pub(crate) output_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DedupeState {
    #[serde(rename = "schemaVersion")]
    schema_version: u8,
    #[serde(default)]
    entries: Vec<CachedHookDecision>,
}

#[derive(Debug, Clone)]
pub(crate) struct DedupeStore {
    path: PathBuf,
}

impl DedupeStore {
    pub(crate) fn new(data_home: &Path) -> Self {
        Self {
            path: data_home.join(DEDUPE_FILE_NAME),
        }
    }

    pub(crate) fn lookup(&self, key_sha256: &str) -> Result<Option<CachedHookDecision>, JevxError> {
        self.with_state(|state, now| {
            let changed = prune_expired(&mut state.entries, now);
            let hit = state
                .entries
                .iter()
                .find(|entry| entry.key_sha256 == key_sha256)
                .cloned();
            Ok((hit, changed))
        })
    }

    pub(crate) fn insert(&self, mut entry: CachedHookDecision) -> Result<(), JevxError> {
        self.with_state(|state, now| {
            prune_expired(&mut state.entries, now);
            entry.created_at_ms = now;
            state
                .entries
                .retain(|existing| existing.key_sha256 != entry.key_sha256);
            state.entries.push(entry.clone());
            if state.entries.len() > DEDUPE_CAPACITY {
                let remove_count = state.entries.len() - DEDUPE_CAPACITY;
                state.entries.drain(..remove_count);
            }
            Ok(((), true))
        })
    }

    fn with_state<T, F>(&self, mut operation: F) -> Result<T, JevxError>
    where
        F: FnMut(&mut DedupeState, u64) -> Result<(T, bool), JevxError>,
    {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&self.path)?;
        file.lock_exclusive()?;

        let result = (|| {
            let mut bytes = Vec::new();
            file.seek(SeekFrom::Start(0))?;
            file.read_to_end(&mut bytes)?;
            let mut state = if bytes.iter().any(|byte| !byte.is_ascii_whitespace()) {
                serde_json::from_slice::<DedupeState>(&bytes).map_err(|_| {
                    JevxError::InvalidInput("hook dedupe state is invalid".to_owned())
                })?
            } else {
                DedupeState {
                    schema_version: DEDUPE_SCHEMA_VERSION,
                    entries: Vec::new(),
                }
            };
            if state.schema_version != DEDUPE_SCHEMA_VERSION {
                return Err(JevxError::InvalidInput(
                    "unsupported hook dedupe state schema".to_owned(),
                ));
            }
            let (value, changed) = operation(&mut state, now_millis())?;
            if changed {
                file.set_len(0)?;
                file.seek(SeekFrom::Start(0))?;
                serde_json::to_writer(&mut file, &state)?;
                file.write_all(b"\n")?;
                file.sync_data()?;
            }
            Ok(value)
        })();
        let unlock = file.unlock();
        match (result, unlock) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(JevxError::from(error)),
        }
    }
}

fn prune_expired(entries: &mut Vec<CachedHookDecision>, now: u64) -> bool {
    let original_len = entries.len();
    entries.retain(|entry| now.saturating_sub(entry.created_at_ms) <= DEDUPE_TTL_MS);
    entries.len() != original_len
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn store_round_trips_and_replaces_a_key() {
        let root = tempdir().expect("tempdir");
        let store = DedupeStore::new(root.path());
        let entry = CachedHookDecision {
            key_sha256: "key".to_owned(),
            created_at_ms: 0,
            decision: CandidateDecision::None,
            selected_skill: None,
            discovery_ms: Some(1),
            jev_response_ms: Some(2),
            total_ms: Some(3),
            input_tokens: Some(4),
            output_tokens: Some(5),
        };
        store.insert(entry.clone()).expect("insert");
        let stored = store.lookup("key").expect("lookup").expect("stored entry");
        assert_eq!(stored.key_sha256, entry.key_sha256);
        assert_eq!(stored.decision, entry.decision);
        assert_eq!(stored.total_ms, entry.total_ms);
        store
            .insert(CachedHookDecision {
                decision: CandidateDecision::Selected,
                ..entry
            })
            .expect("replace");
        assert_eq!(
            store.lookup("key").expect("lookup").expect("hit").decision,
            CandidateDecision::Selected
        );
    }

    #[test]
    fn corrupt_state_fails_closed_to_no_cache() {
        let root = tempdir().expect("tempdir");
        let path = root.path().join(DEDUPE_FILE_NAME);
        fs::write(&path, "not-json\n").expect("corrupt state");
        assert!(DedupeStore::new(root.path()).lookup("key").is_err());
    }

    #[test]
    fn state_rejects_unknown_schema_and_caps_old_entries() {
        let root = tempdir().expect("tempdir");
        let nested = root.path().join("nested");
        fs::create_dir_all(&nested).expect("nested dir");
        fs::write(
            nested.join(DEDUPE_FILE_NAME),
            r#"{"schemaVersion":99,"entries":[]}"#,
        )
        .expect("unsupported state");
        assert!(DedupeStore::new(&nested).lookup("key").is_err());

        let expired_root = root.path().join("expired");
        fs::create_dir_all(&expired_root).expect("expired dir");
        let expired_entry = CachedHookDecision {
            key_sha256: "expired-key".to_owned(),
            created_at_ms: 1,
            decision: CandidateDecision::None,
            selected_skill: None,
            discovery_ms: None,
            jev_response_ms: None,
            total_ms: None,
            input_tokens: None,
            output_tokens: None,
        };
        let expired_state = DedupeState {
            schema_version: DEDUPE_SCHEMA_VERSION,
            entries: vec![expired_entry],
        };
        fs::write(
            expired_root.join(DEDUPE_FILE_NAME),
            serde_json::to_vec(&expired_state).expect("expired state json"),
        )
        .expect("expired state");
        assert!(
            DedupeStore::new(&expired_root)
                .lookup("expired-key")
                .expect("expired lookup")
                .is_none()
        );

        let store = DedupeStore::new(root.path());
        for index in 0..=DEDUPE_CAPACITY {
            store
                .insert(CachedHookDecision {
                    key_sha256: format!("key-{index}"),
                    created_at_ms: 0,
                    decision: CandidateDecision::None,
                    selected_skill: None,
                    discovery_ms: None,
                    jev_response_ms: None,
                    total_ms: None,
                    input_tokens: None,
                    output_tokens: None,
                })
                .expect("insert entry");
        }
        assert!(store.lookup("key-0").expect("lookup oldest").is_none());
        assert!(
            store
                .lookup(&format!("key-{DEDUPE_CAPACITY}"))
                .expect("lookup newest")
                .is_some()
        );
    }
}
