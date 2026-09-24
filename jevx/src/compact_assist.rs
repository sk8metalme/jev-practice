use std::fs;
use std::fs::OpenOptions;
use std::io::Read;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Config;
use crate::JevxError;
use crate::cost::CostEstimate;
use crate::hooks::{
    HookShadowRecord, TokenSavings, TokenUsageSnapshot, append_shadow_record, hook_event_name,
    run_shadow,
};
use crate::redaction::{redact, sha256_hex};
use crate::storage::append_json_line;

const CONTEXT_LIMIT: usize = 4_000;
const CONTEXT_READ_LIMIT_BYTES: u64 = CONTEXT_LIMIT as u64 * 4 + 1;
pub(crate) const COMPACT_CHECKPOINT_SCHEMA_VERSION: u8 = 4;
pub(crate) const COMPACT_ASSIST_LOCK_FILE_NAME: &str = ".compact-assist.lock";

#[derive(Debug, Clone, Serialize)]
pub struct CompactAssistResult {
    pub response: Value,
    pub checkpoint: CompactCheckpoint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactCheckpoint {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub mode: String,
    #[serde(rename = "hookEventName")]
    pub hook_event_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(rename = "sessionIdSha256", skip_serializing_if = "Option::is_none")]
    pub session_id_sha256: Option<String>,
    #[serde(rename = "turnIdSha256", skip_serializing_if = "Option::is_none")]
    pub turn_id_sha256: Option<String>,
    #[serde(
        rename = "correlationIdSha256",
        skip_serializing_if = "Option::is_none"
    )]
    pub correlation_id_sha256: Option<String>,
    #[serde(rename = "cwdSha256", skip_serializing_if = "Option::is_none")]
    pub cwd_sha256: Option<String>,
    #[serde(rename = "contextSha256", skip_serializing_if = "Option::is_none")]
    pub context_sha256: Option<String>,
    #[serde(rename = "contextChars")]
    pub context_chars: usize,
    #[serde(rename = "contextAvailable")]
    pub context_available: bool,
    #[serde(rename = "preCompactionUsage", skip_serializing_if = "Option::is_none")]
    pub pre_compaction_usage: Option<TokenUsageSnapshot>,
    #[serde(rename = "compactionUsage", skip_serializing_if = "Option::is_none")]
    pub compaction_usage: Option<TokenUsageSnapshot>,
    #[serde(
        rename = "postCompactionUsage",
        skip_serializing_if = "Option::is_none"
    )]
    pub post_compaction_usage: Option<TokenUsageSnapshot>,
    #[serde(rename = "postCompactionCost", skip_serializing_if = "Option::is_none")]
    pub post_compaction_cost: Option<CostEstimate>,
    #[serde(
        rename = "compactionElapsedMs",
        skip_serializing_if = "Option::is_none"
    )]
    pub compaction_elapsed_ms: Option<u64>,
    #[serde(rename = "tokenSavings", skip_serializing_if = "Option::is_none")]
    pub token_savings: Option<TokenSavings>,
}

#[derive(Debug, Clone)]
struct ContextSnapshot {
    redacted: String,
    hash: String,
    chars: usize,
}

pub async fn run_compact_assist(
    input: &str,
    config: &Config,
    state_dir: &Path,
) -> Result<CompactAssistResult, JevxError> {
    let payload: Value = serde_json::from_str(input)?;
    let mut shadow = run_shadow(input, None, &[], config, None).await?;
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let context = read_context(&cwd)?;
    fs::create_dir_all(state_dir)?;
    let (prior_checkpoint, checkpoint) = with_compaction_lock(state_dir, || {
        let prior_checkpoint = if is_compact_session_start(&payload) {
            latest_checkpoint(state_dir, shadow.record.session_id_sha256.as_deref())?
        } else {
            None
        };
        let previous_pre_checkpoint = if is_post_compact(&payload) {
            latest_pre_compaction_checkpoint(
                state_dir,
                shadow.record.session_id_sha256.as_deref(),
                shadow.record.turn_id_sha256.as_deref(),
                shadow.record.correlation_id_sha256.as_deref(),
            )?
        } else {
            None
        };
        merge_previous_usage(&mut shadow.record, previous_pre_checkpoint.as_ref());
        let checkpoint = checkpoint_from(&payload, &shadow.record, &cwd, context.as_ref());

        append_shadow_record(&state_dir.join("hook-records.jsonl"), &shadow.record)?;
        append_checkpoint(&state_dir.join("checkpoints.jsonl"), &checkpoint)?;
        Ok((prior_checkpoint, checkpoint))
    })?;

    let response = if is_compact_session_start(&payload) {
        session_start_response(prior_checkpoint.as_ref(), context.as_ref())
    } else {
        serde_json::to_value(shadow.response)?
    };

    Ok(CompactAssistResult {
        response,
        checkpoint,
    })
}

fn with_compaction_lock<T, F>(state_dir: &Path, operation: F) -> Result<T, JevxError>
where
    F: FnOnce() -> Result<T, JevxError>,
{
    let lock_path = state_dir.join(COMPACT_ASSIST_LOCK_FILE_NAME);
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)?;
    lock.lock_exclusive()?;
    let result = operation();
    let unlock = lock.unlock();
    match (result, unlock) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(JevxError::from(error)),
    }
}

fn checkpoint_from(
    payload: &Value,
    record: &HookShadowRecord,
    cwd: &Path,
    context: Option<&ContextSnapshot>,
) -> CompactCheckpoint {
    CompactCheckpoint {
        schema_version: COMPACT_CHECKPOINT_SCHEMA_VERSION,
        mode: "compact-assist".to_owned(),
        hook_event_name: record.hook_event_name.clone(),
        trigger: safe_label(record.trigger.as_deref(), 32),
        source: safe_label(record.source.as_deref(), 64),
        session_id_sha256: record.session_id_sha256.clone(),
        turn_id_sha256: record.turn_id_sha256.clone(),
        correlation_id_sha256: record.correlation_id_sha256.clone(),
        cwd_sha256: payload
            .get("cwd")
            .and_then(Value::as_str)
            .or_else(|| cwd.to_str())
            .map(sha256_hex),
        context_sha256: context.map(|value| value.hash.clone()),
        context_chars: context.map_or(0, |value| value.chars),
        context_available: context.is_some(),
        pre_compaction_usage: record.pre_compaction_usage.clone(),
        compaction_usage: record.compaction_usage.clone(),
        post_compaction_usage: record.post_compaction_usage.clone(),
        post_compaction_cost: record.post_compaction_cost.clone(),
        compaction_elapsed_ms: record.compaction_elapsed_ms,
        token_savings: record.token_savings.clone(),
    }
}

fn merge_previous_usage(record: &mut HookShadowRecord, previous: Option<&CompactCheckpoint>) {
    let Some(previous) = previous else {
        return;
    };
    if !same_compaction_cycle(record, previous) {
        return;
    }
    if record.pre_compaction_usage.is_none() {
        record.pre_compaction_usage = previous.pre_compaction_usage.clone();
    }
    if record.compaction_usage.is_none() {
        record.compaction_usage = previous.compaction_usage.clone();
    }
    if let (Some(before), Some(after)) = (
        record.pre_compaction_usage.as_ref(),
        record.post_compaction_usage.as_ref(),
    ) {
        record.token_savings = Some(TokenSavings::from_snapshots(
            Some(before),
            Some(after),
            "hook_checkpoint",
        ));
    }
}

fn same_compaction_cycle(record: &HookShadowRecord, previous: &CompactCheckpoint) -> bool {
    same_known_cycle(
        record.turn_id_sha256.as_deref(),
        record.correlation_id_sha256.as_deref(),
        previous.turn_id_sha256.as_deref(),
        previous.correlation_id_sha256.as_deref(),
    )
}

fn same_checkpoint_cycle(left: &CompactCheckpoint, right: &CompactCheckpoint) -> bool {
    same_known_cycle(
        left.turn_id_sha256.as_deref(),
        left.correlation_id_sha256.as_deref(),
        right.turn_id_sha256.as_deref(),
        right.correlation_id_sha256.as_deref(),
    )
}

fn same_known_cycle(
    left_turn: Option<&str>,
    left_correlation: Option<&str>,
    right_turn: Option<&str>,
    right_correlation: Option<&str>,
) -> bool {
    matches!((left_turn, right_turn), (Some(left), Some(right)) if left == right)
        && same_optional_identity(left_correlation, right_correlation)
}

fn same_optional_identity(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left == right,
        (None, None) => true,
        _ => false,
    }
}

fn append_checkpoint(path: &Path, checkpoint: &CompactCheckpoint) -> Result<(), JevxError> {
    append_json_line(path, checkpoint)
}

fn latest_checkpoint(
    state_dir: &Path,
    session_id_sha256: Option<&str>,
) -> Result<Option<CompactCheckpoint>, JevxError> {
    latest_checkpoint_where(state_dir, session_id_sha256, |event| {
        matches!(event, "PreCompact" | "PostCompact")
    })
}

fn latest_pre_compaction_checkpoint(
    state_dir: &Path,
    session_id_sha256: Option<&str>,
    turn_id_sha256: Option<&str>,
    correlation_id_sha256: Option<&str>,
) -> Result<Option<CompactCheckpoint>, JevxError> {
    let Some(session_id_sha256) = session_id_sha256 else {
        return Ok(None);
    };
    let path = state_dir.join("checkpoints.jsonl");
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path)?;
    let mut pending = Vec::new();
    for (line_number, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let checkpoint = parse_checkpoint(line, line_number + 1)?;
        if checkpoint.session_id_sha256.as_deref() != Some(session_id_sha256) {
            continue;
        }
        match checkpoint.hook_event_name.as_str() {
            "PreCompact" => pending.push(checkpoint),
            "PostCompact" => {
                if let Some(index) = pending
                    .iter()
                    .rposition(|pre| same_checkpoint_cycle(pre, &checkpoint))
                {
                    pending.remove(index);
                }
            }
            _ => {}
        }
    }
    Ok(pending.into_iter().rev().find(|pre| {
        same_known_cycle(
            pre.turn_id_sha256.as_deref(),
            pre.correlation_id_sha256.as_deref(),
            turn_id_sha256,
            correlation_id_sha256,
        )
    }))
}

fn latest_checkpoint_where<F>(
    state_dir: &Path,
    session_id_sha256: Option<&str>,
    event_matches: F,
) -> Result<Option<CompactCheckpoint>, JevxError>
where
    F: Fn(&str) -> bool,
{
    let Some(session_id_sha256) = session_id_sha256 else {
        return Ok(None);
    };
    let path = state_dir.join("checkpoints.jsonl");
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path)?;
    let mut latest = None;
    for (line_number, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let checkpoint = parse_checkpoint(line, line_number + 1)?;
        if checkpoint.session_id_sha256.as_deref() == Some(session_id_sha256)
            && event_matches(checkpoint.hook_event_name.as_str())
        {
            latest = Some(checkpoint);
        }
    }
    Ok(latest)
}

fn parse_checkpoint(line: &str, line_number: usize) -> Result<CompactCheckpoint, JevxError> {
    let mut checkpoint = serde_json::from_str::<CompactCheckpoint>(line).map_err(|_| {
        JevxError::InvalidInput(format!(
            "invalid compaction checkpoint at line {line_number}"
        ))
    })?;
    let schema_version = checkpoint.schema_version;
    if !matches!(
        checkpoint.schema_version,
        1 | 2 | 3 | COMPACT_CHECKPOINT_SCHEMA_VERSION
    ) {
        return Err(JevxError::InvalidInput(format!(
            "unsupported compaction checkpoint schema at line {line_number}"
        )));
    }
    if schema_version < COMPACT_CHECKPOINT_SCHEMA_VERSION {
        for usage in [
            checkpoint.pre_compaction_usage.as_mut(),
            checkpoint.compaction_usage.as_mut(),
            checkpoint.post_compaction_usage.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            usage.normalize_legacy_presence();
        }
    }
    checkpoint.schema_version = COMPACT_CHECKPOINT_SCHEMA_VERSION;
    Ok(checkpoint)
}

fn read_context(cwd: &Path) -> Result<Option<ContextSnapshot>, JevxError> {
    let path = cwd.join(".jevx/compact-context.md");
    if !path.exists() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(CONTEXT_READ_LIMIT_BYTES)
        .read_to_end(&mut bytes)?;
    let raw = String::from_utf8_lossy(&bytes);
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let redacted = truncate(&redact(trimmed), CONTEXT_LIMIT);
    Ok(Some(ContextSnapshot {
        hash: sha256_hex(&redacted),
        chars: redacted.chars().count(),
        redacted,
    }))
}

fn session_start_response(
    checkpoint: Option<&CompactCheckpoint>,
    context: Option<&ContextSnapshot>,
) -> Value {
    let mut lines = vec![
        "jevx compact checkpoint restored (deterministic prototype).".to_owned(),
        "Treat this as supplemental context; verify the current repository and conversation."
            .to_owned(),
    ];
    if let Some(checkpoint) = checkpoint {
        lines.push(format!(
            "checkpoint event={} contextAvailable={} contextChars={}",
            checkpoint.hook_event_name, checkpoint.context_available, checkpoint.context_chars
        ));
    } else {
        lines.push("checkpoint metadata was not found for this session.".to_owned());
    }
    if let Some(context) = context {
        lines.push("Local compact context manifest (redacted):".to_owned());
        lines.push(context.redacted.clone());
    }
    json!({
        "continue": true,
        "suppressOutput": true,
        "hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": lines.join("\n")
        }
    })
}

fn is_compact_session_start(payload: &Value) -> bool {
    hook_event_name(payload) == Some("SessionStart")
        && payload.get("source").and_then(Value::as_str) == Some("compact")
}

fn is_post_compact(payload: &Value) -> bool {
    hook_event_name(payload) == Some("PostCompact")
}

fn safe_label(value: Option<&str>, max_chars: usize) -> Option<String> {
    value
        .filter(|value| {
            !value.is_empty()
                && value.chars().count() <= max_chars
                && value
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character))
        })
        .map(str::to_owned)
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut result = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        result.push_str("\n[jevx context truncated]");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    fn test_checkpoint(
        schema_version: u8,
        hook_event_name: &str,
        turn_id_sha256: Option<&str>,
    ) -> CompactCheckpoint {
        CompactCheckpoint {
            schema_version,
            mode: "compact-assist".to_owned(),
            hook_event_name: hook_event_name.to_owned(),
            trigger: Some("manual".to_owned()),
            source: None,
            session_id_sha256: Some("session-hash".to_owned()),
            turn_id_sha256: turn_id_sha256.map(str::to_owned),
            correlation_id_sha256: turn_id_sha256.map(|_| "cycle-hash".to_owned()),
            cwd_sha256: None,
            context_sha256: None,
            context_chars: 0,
            context_available: false,
            pre_compaction_usage: None,
            compaction_usage: None,
            post_compaction_usage: None,
            post_compaction_cost: None,
            compaction_elapsed_ms: None,
            token_savings: None,
        }
    }

    #[test]
    fn helpers_filter_unsafe_labels_and_truncate_context() {
        assert_eq!(safe_label(Some("manual"), 32).as_deref(), Some("manual"));
        assert!(safe_label(Some("manual value"), 32).is_none());
        assert!(safe_label(Some(""), 32).is_none());
        assert_eq!(truncate("abcdef", 3), "abc\n[jevx context truncated]");
        assert_eq!(truncate("abc", 3), "abc");
        assert!(!is_compact_session_start(&json!({
            "hook_event_name": "SessionStart",
            "source": "startup"
        })));
        assert!(is_post_compact(&json!({"event": "PostCompact"})));
    }

    #[test]
    fn checkpoint_and_context_helpers_handle_missing_invalid_and_empty_state() {
        let root = tempdir().expect("tempdir");
        assert!(
            latest_checkpoint(&root.path().join("missing"), None)
                .expect("missing")
                .is_none()
        );

        let checkpoint = test_checkpoint(
            COMPACT_CHECKPOINT_SCHEMA_VERSION,
            "PreCompact",
            Some("turn-hash"),
        );
        let checkpoint_path = root.path().join("nested/checkpoints.jsonl");
        append_checkpoint(&checkpoint_path, &checkpoint).expect("checkpoint");
        let mut post_checkpoint = checkpoint.clone();
        post_checkpoint.hook_event_name = "PostCompact".to_owned();
        append_checkpoint(&checkpoint_path, &post_checkpoint).expect("post checkpoint");
        fs::OpenOptions::new()
            .append(true)
            .open(&checkpoint_path)
            .expect("open checkpoint")
            .write_all(b"{not-json}\n")
            .expect("invalid checkpoint");
        assert!(
            latest_checkpoint(
                checkpoint_path.parent().expect("state dir"),
                Some("session-hash")
            )
            .is_err()
        );
        assert!(
            latest_pre_compaction_checkpoint(
                checkpoint_path.parent().expect("state dir"),
                Some("session-hash"),
                Some("turn-hash"),
                Some("cycle-hash"),
            )
            .is_err()
        );
        let clean_state_dir = root.path().join("clean-state");
        let clean_checkpoint_path = clean_state_dir.join("checkpoints.jsonl");
        append_checkpoint(&clean_checkpoint_path, &checkpoint).expect("next pre checkpoint");
        assert_eq!(
            latest_pre_compaction_checkpoint(
                clean_checkpoint_path.parent().expect("state dir"),
                Some("session-hash"),
                Some("turn-hash"),
                Some("cycle-hash"),
            )
            .expect("latest pre")
            .expect("next pre checkpoint")
            .hook_event_name,
            "PreCompact"
        );
        append_checkpoint(&clean_checkpoint_path, &post_checkpoint).expect("clean post");
        assert!(
            latest_pre_compaction_checkpoint(
                clean_checkpoint_path.parent().expect("state dir"),
                Some("session-hash"),
                Some("turn-hash"),
                Some("cycle-hash"),
            )
            .expect("latest pre after post")
            .is_none()
        );
        assert!(
            latest_checkpoint(
                clean_checkpoint_path.parent().expect("state dir"),
                Some("other-session")
            )
            .expect("other session")
            .is_none()
        );

        assert!(
            read_context(root.path())
                .expect("missing context")
                .is_none()
        );
        fs::create_dir_all(root.path().join(".jevx")).expect("context dir");
        fs::write(root.path().join(".jevx/compact-context.md"), "  \n").expect("empty context");
        assert!(read_context(root.path()).expect("empty context").is_none());

        let response = session_start_response(None, None);
        assert!(
            response["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .expect("context")
                .contains("not found")
        );
    }

    #[test]
    fn checkpoint_reads_migrate_supported_versions_and_reject_unknown_versions() {
        let root = tempdir().expect("tempdir");
        for version in [1, 2, 3] {
            let legacy_root = root.path().join(format!("legacy-v{version}"));
            let legacy = test_checkpoint(version, "PreCompact", Some("turn-hash"));
            append_checkpoint(&legacy_root.join("checkpoints.jsonl"), &legacy)
                .expect("legacy checkpoint");
            let migrated = latest_checkpoint(&legacy_root, Some("session-hash"))
                .expect("supported legacy schema")
                .expect("legacy checkpoint found");
            assert_eq!(migrated.schema_version, COMPACT_CHECKPOINT_SCHEMA_VERSION);
        }

        let future_root = root.path().join("future");
        let future = test_checkpoint(99, "PreCompact", Some("turn-hash"));
        append_checkpoint(&future_root.join("checkpoints.jsonl"), &future)
            .expect("future checkpoint");
        assert!(latest_checkpoint(&future_root, Some("session-hash")).is_err());
    }

    #[test]
    fn legacy_v3_checkpoint_distinguishes_missing_total_from_explicit_zero() {
        let root = tempdir().expect("tempdir");
        let missing_root = root.path().join("missing-total");
        fs::create_dir_all(&missing_root).expect("missing-total state directory");
        let mut missing = serde_json::to_value(test_checkpoint(3, "PreCompact", Some("turn-hash")))
            .expect("legacy checkpoint json");
        missing["preCompactionUsage"] = serde_json::json!({
            "inputTokens": 900,
            "cachedInputTokens": 0,
            "cacheWriteInputTokens": 0,
            "outputTokens": 0,
            "reasoningOutputTokens": 0,
            "totalTokens": 0
        });
        fs::write(
            missing_root.join("checkpoints.jsonl"),
            format!("{}\n", missing),
        )
        .expect("legacy missing-total checkpoint");
        let migrated = latest_checkpoint(&missing_root, Some("session-hash"))
            .expect("legacy checkpoint read")
            .expect("checkpoint found");
        assert_eq!(migrated.schema_version, COMPACT_CHECKPOINT_SCHEMA_VERSION);
        let migrated_usage = migrated.pre_compaction_usage.as_ref().expect("pre usage");
        assert!(
            !migrated_usage.total_tokens_present && migrated_usage.total_tokens == 0,
            "legacy serializer's default zero is not measured usage"
        );

        let explicit_root = root.path().join("explicit-zero");
        fs::create_dir_all(&explicit_root).expect("explicit-zero state directory");
        let mut explicit =
            serde_json::to_value(test_checkpoint(3, "PreCompact", Some("turn-hash")))
                .expect("legacy checkpoint json");
        explicit["preCompactionUsage"] = serde_json::json!({
            "inputTokens": 0,
            "cachedInputTokens": 0,
            "cacheWriteInputTokens": 0,
            "outputTokens": 0,
            "reasoningOutputTokens": 0,
            "totalTokens": 0,
            "totalTokensPresent": true
        });
        fs::write(
            explicit_root.join("checkpoints.jsonl"),
            format!("{}\n", explicit),
        )
        .expect("legacy explicit-zero checkpoint");
        let migrated = latest_checkpoint(&explicit_root, Some("session-hash"))
            .expect("legacy checkpoint read")
            .expect("checkpoint found");
        assert!(
            migrated
                .pre_compaction_usage
                .as_ref()
                .expect("pre usage")
                .total_tokens_present,
            "legacy explicit zero marker remains measured usage"
        );
    }

    #[test]
    fn pre_compaction_checkpoint_without_turn_identity_is_not_measured() {
        let root = tempdir().expect("tempdir");
        let pre = test_checkpoint(COMPACT_CHECKPOINT_SCHEMA_VERSION, "PreCompact", None);
        append_checkpoint(&root.path().join("checkpoints.jsonl"), &pre).expect("pre checkpoint");
        assert!(
            latest_pre_compaction_checkpoint(root.path(), Some("session-hash"), None, None)
                .expect("lookup")
                .is_none()
        );
    }

    #[test]
    fn context_reader_bounds_invalid_utf8_after_the_context_budget() {
        let root = tempdir().expect("tempdir");
        fs::create_dir_all(root.path().join(".jevx")).expect("context dir");
        let mut content = vec![b'a'; CONTEXT_READ_LIMIT_BYTES as usize + 100];
        content[CONTEXT_READ_LIMIT_BYTES as usize] = 0xff;
        fs::write(root.path().join(".jevx/compact-context.md"), content).expect("context");

        let snapshot = read_context(root.path())
            .expect("bounded context read")
            .expect("context snapshot");
        assert!(snapshot.redacted.contains("[jevx context truncated]"));
        assert!(snapshot.redacted.chars().count() < CONTEXT_READ_LIMIT_BYTES as usize);
    }
}
