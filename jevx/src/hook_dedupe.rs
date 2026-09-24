use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::JevxError;
use crate::types::CandidateDecision;

pub(crate) const DEDUPE_FILE_NAME: &str = "hook-dedupe.json";
pub(crate) const DEDUPE_LOCK_FILE_NAME: &str = "hook-dedupe.lock";
const DEDUPE_SCHEMA_VERSION: u8 = 2;
const PREVIOUS_DEDUPE_SCHEMA_VERSION: u8 = 1;
const DEDUPE_TTL_MS: u64 = 30_000;
const DEDUPE_CAPACITY: usize = 256;
pub(crate) const DEDUPE_TEMP_DIR_NAME: &str = ".hook-dedupe-tmp";
const DEDUPE_PENDING_GRACE_MS: u64 = 1_000;

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
    #[serde(default)]
    pending: Vec<PendingHookDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct PendingHookDecision {
    #[serde(rename = "keySha256")]
    key_sha256: String,
    #[serde(rename = "createdAtMs")]
    created_at_ms: u64,
}

impl DedupeState {
    fn empty() -> Self {
        Self {
            schema_version: DEDUPE_SCHEMA_VERSION,
            entries: Vec::new(),
            pending: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DedupeClaim {
    Hit(CachedHookDecision),
    Owner,
    Wait,
}

#[derive(Debug, Clone)]
pub(crate) struct DedupeStore {
    path: PathBuf,
    lock_path: PathBuf,
    pending_ttl_ms: u64,
}

impl DedupeStore {
    #[cfg(test)]
    pub(crate) fn new(data_home: &Path) -> Self {
        Self::with_pending_ttl(data_home, Duration::from_millis(DEDUPE_TTL_MS))
    }

    pub(crate) fn with_pending_ttl(data_home: &Path, timeout: Duration) -> Self {
        Self {
            path: data_home.join(DEDUPE_FILE_NAME),
            lock_path: data_home.join(DEDUPE_LOCK_FILE_NAME),
            pending_ttl_ms: timeout
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX)
                .saturating_add(DEDUPE_PENDING_GRACE_MS),
        }
    }

    pub(crate) fn claim(&self, key_sha256: &str) -> Result<DedupeClaim, JevxError> {
        self.with_state(|state, now| {
            let mut changed = prune_expired(
                &mut state.entries,
                &mut state.pending,
                now,
                self.pending_ttl_ms,
            );
            if state
                .pending
                .iter()
                .any(|pending| pending.key_sha256 == key_sha256)
            {
                return Ok((DedupeClaim::Wait, changed));
            }
            if let Some(entry) = state
                .entries
                .iter()
                .find(|entry| entry.key_sha256 == key_sha256)
                .cloned()
            {
                return Ok((DedupeClaim::Hit(entry), changed));
            }
            state.pending.push(PendingHookDecision {
                key_sha256: key_sha256.to_owned(),
                created_at_ms: now,
            });
            trim_pending(&mut state.pending);
            changed = true;
            Ok((DedupeClaim::Owner, changed))
        })
    }

    #[cfg(test)]
    pub(crate) fn lookup(&self, key_sha256: &str) -> Result<Option<CachedHookDecision>, JevxError> {
        self.with_state(|state, now| {
            let changed = prune_expired(
                &mut state.entries,
                &mut state.pending,
                now,
                self.pending_ttl_ms,
            );
            let hit = state
                .entries
                .iter()
                .find(|entry| entry.key_sha256 == key_sha256)
                .cloned();
            Ok((hit, changed))
        })
    }

    #[cfg(test)]
    pub(crate) fn insert(&self, mut entry: CachedHookDecision) -> Result<(), JevxError> {
        self.with_state(|state, now| {
            prune_expired(
                &mut state.entries,
                &mut state.pending,
                now,
                self.pending_ttl_ms,
            );
            state
                .pending
                .retain(|pending| pending.key_sha256 != entry.key_sha256);
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

    pub(crate) fn complete(
        &self,
        key_sha256: &str,
        decision: CandidateDecision,
        selected_skill: Option<String>,
    ) -> Result<(), JevxError> {
        self.with_state(|state, now| {
            state
                .pending
                .retain(|pending| pending.key_sha256 != key_sha256);
            if let Some(entry) = state
                .entries
                .iter_mut()
                .find(|entry| entry.key_sha256 == key_sha256)
            {
                entry.created_at_ms = now;
                entry.decision = decision.clone();
                entry.selected_skill = selected_skill.clone();
                entry.discovery_ms = None;
                entry.jev_response_ms = None;
                entry.total_ms = None;
                entry.input_tokens = None;
                entry.output_tokens = None;
            } else {
                state.entries.push(CachedHookDecision {
                    key_sha256: key_sha256.to_owned(),
                    created_at_ms: now,
                    decision: decision.clone(),
                    selected_skill: selected_skill.clone(),
                    discovery_ms: None,
                    jev_response_ms: None,
                    total_ms: None,
                    input_tokens: None,
                    output_tokens: None,
                });
            }
            trim_entries(&mut state.entries);
            Ok(((), true))
        })
    }

    pub(crate) fn release(&self, key_sha256: &str) -> Result<(), JevxError> {
        self.with_state(|state, now| {
            let before = state.pending.len();
            state
                .pending
                .retain(|pending| pending.key_sha256 != key_sha256);
            let changed = before != state.pending.len();
            let changed_by_prune = prune_expired(
                &mut state.entries,
                &mut state.pending,
                now,
                self.pending_ttl_ms,
            );
            Ok(((), changed || changed_by_prune))
        })
    }

    fn with_state<T, F>(&self, mut operation: F) -> Result<T, JevxError>
    where
        F: FnMut(&mut DedupeState, u64) -> Result<(T, bool), JevxError>,
    {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&self.lock_path)?;
        lock.lock_exclusive()?;

        let result = (|| {
            let mut file = OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .open(&self.path)?;
            let mut bytes = Vec::new();
            file.seek(SeekFrom::Start(0))?;
            file.read_to_end(&mut bytes)?;
            drop(file);
            let (mut state, migrated) = if bytes.iter().any(|byte| !byte.is_ascii_whitespace()) {
                let mut state = serde_json::from_slice::<DedupeState>(&bytes).map_err(|_| {
                    JevxError::InvalidInput("hook dedupe state is invalid".to_owned())
                })?;
                let migrated = state.schema_version == PREVIOUS_DEDUPE_SCHEMA_VERSION;
                if migrated {
                    state.schema_version = DEDUPE_SCHEMA_VERSION;
                }
                (state, migrated)
            } else {
                (DedupeState::empty(), false)
            };
            if state.schema_version != DEDUPE_SCHEMA_VERSION {
                return Err(JevxError::InvalidInput(
                    "unsupported hook dedupe state schema".to_owned(),
                ));
            }
            let (value, changed) = operation(&mut state, now_millis())?;
            if changed || migrated {
                atomic_write_state(&self.path, &state)?;
            }
            Ok(value)
        })();
        let unlock = lock.unlock();
        match (result, unlock) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(JevxError::from(error)),
        }
    }
}

fn atomic_write_state(path: &Path, state: &DedupeState) -> Result<(), JevxError> {
    let Some(parent) = path.parent() else {
        return Err(JevxError::InvalidInput(
            "hook dedupe state path must have a parent".to_owned(),
        ));
    };
    let mut bytes = serde_json::to_vec(state)?;
    bytes.push(b'\n');
    let base = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("hook-dedupe.json");
    let temp_dir = parent.join(DEDUPE_TEMP_DIR_NAME);
    fs::create_dir_all(&temp_dir)?;
    for attempt in 0..100_u32 {
        let temp = temp_dir.join(format!(".{base}.{}.{}.tmp", std::process::id(), attempt));
        let mut file = match OpenOptions::new().create_new(true).write(true).open(&temp) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        };
        let result = (|| {
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temp, path)?;
            Ok::<(), std::io::Error>(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        return result.map_err(JevxError::from);
    }
    Err(std::io::Error::new(
        ErrorKind::AlreadyExists,
        "could not allocate temporary hook dedupe state path",
    )
    .into())
}

fn prune_expired(
    entries: &mut Vec<CachedHookDecision>,
    pending: &mut Vec<PendingHookDecision>,
    now: u64,
    pending_ttl_ms: u64,
) -> bool {
    let original_len = entries.len();
    entries.retain(|entry| now.saturating_sub(entry.created_at_ms) <= DEDUPE_TTL_MS);
    let original_pending_len = pending.len();
    pending.retain(|entry| now.saturating_sub(entry.created_at_ms) <= pending_ttl_ms);
    entries.len() != original_len || pending.len() != original_pending_len
}

fn trim_entries(entries: &mut Vec<CachedHookDecision>) {
    if entries.len() > DEDUPE_CAPACITY {
        let remove_count = entries.len() - DEDUPE_CAPACITY;
        entries.drain(..remove_count);
    }
}

fn trim_pending(pending: &mut Vec<PendingHookDecision>) {
    if pending.len() > DEDUPE_CAPACITY {
        let remove_count = pending.len() - DEDUPE_CAPACITY;
        pending.drain(..remove_count);
    }
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
    fn claim_serializes_in_flight_work_and_releases_failures() {
        let root = tempdir().expect("tempdir");
        let store = DedupeStore::new(root.path());
        assert_eq!(store.claim("key").expect("owner"), DedupeClaim::Owner);
        assert_eq!(store.claim("key").expect("waiter"), DedupeClaim::Wait);
        store
            .complete("key", CandidateDecision::Selected, Some("pdf".to_owned()))
            .expect("complete");
        store
            .complete("key", CandidateDecision::None, None)
            .expect("replace completed entry");
        match store.claim("key").expect("hit") {
            DedupeClaim::Hit(entry) => {
                assert_eq!(entry.decision, CandidateDecision::None);
                assert_eq!(entry.selected_skill, None);
                assert_eq!(entry.discovery_ms, None);
                assert_eq!(entry.output_tokens, None);
            }
            other => panic!("expected hit, got {other:?}"),
        }

        assert_eq!(
            store.claim("failed-key").expect("owner"),
            DedupeClaim::Owner
        );
        store.release("failed-key").expect("release");
        assert_eq!(
            store.claim("failed-key").expect("reclaimed owner"),
            DedupeClaim::Owner
        );
    }

    #[test]
    fn pending_claims_are_bounded() {
        let mut pending = (0..=DEDUPE_CAPACITY)
            .map(|index| PendingHookDecision {
                key_sha256: format!("key-{index}"),
                created_at_ms: 0,
            })
            .collect::<Vec<_>>();
        trim_pending(&mut pending);
        assert_eq!(pending.len(), DEDUPE_CAPACITY);
        assert_eq!(pending.first().expect("oldest pending").key_sha256, "key-1");
    }

    #[test]
    fn pending_claim_lease_uses_evaluation_timeout_not_cache_ttl() {
        let mut entries = Vec::new();
        let mut pending = vec![PendingHookDecision {
            key_sha256: "long-running".to_owned(),
            created_at_ms: 0,
        }];
        assert!(!prune_expired(&mut entries, &mut pending, 30_001, 60_000));
        assert_eq!(pending.len(), 1);
        assert!(prune_expired(&mut entries, &mut pending, 60_001, 60_000));
        assert!(pending.is_empty());
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
            pending: Vec::new(),
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

        let legacy_root = root.path().join("legacy");
        fs::create_dir_all(&legacy_root).expect("legacy dir");
        fs::write(
            legacy_root.join(DEDUPE_FILE_NAME),
            r#"{"schemaVersion":1,"entries":[]}"#,
        )
        .expect("legacy state");
        assert_eq!(
            DedupeStore::new(&legacy_root)
                .claim("legacy-key")
                .expect("legacy migration"),
            DedupeClaim::Owner
        );
        let migrated: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(legacy_root.join(DEDUPE_FILE_NAME)).expect("migrated state"),
        )
        .expect("migrated json");
        assert_eq!(migrated["schemaVersion"], DEDUPE_SCHEMA_VERSION);
    }
}
