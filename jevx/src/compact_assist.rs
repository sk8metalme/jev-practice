use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Config;
use crate::JevxError;
use crate::cost::CostEstimate;
use crate::hooks::{
    HookShadowRecord, TokenSavings, TokenUsageSnapshot, append_shadow_record, run_shadow,
};
use crate::redaction::{redact, sha256_hex};
use crate::storage::append_json_line;

const CONTEXT_LIMIT: usize = 4_000;
const CONTEXT_READ_LIMIT_BYTES: u64 = CONTEXT_LIMIT as u64 * 4 + 1;
const COMPACT_CHECKPOINT_SCHEMA_VERSION: u8 = 2;

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
    let prior_checkpoint = if is_compact_session_start(&payload) {
        latest_checkpoint(state_dir, shadow.record.session_id_sha256.as_deref())?
    } else {
        None
    };
    let previous_pre_checkpoint = if is_post_compact(&payload) {
        latest_pre_compaction_checkpoint(state_dir, shadow.record.session_id_sha256.as_deref())?
    } else {
        None
    };
    merge_previous_usage(&mut shadow.record, previous_pre_checkpoint.as_ref());
    let checkpoint = checkpoint_from(&payload, &shadow.record, &cwd, context.as_ref());

    fs::create_dir_all(state_dir)?;
    append_shadow_record(&state_dir.join("hook-records.jsonl"), &shadow.record)?;
    append_checkpoint(&state_dir.join("checkpoints.jsonl"), &checkpoint)?;

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
    if record.pre_compaction_usage.is_none() {
        record.pre_compaction_usage = previous.pre_compaction_usage.clone();
    }
    if record.compaction_usage.is_none() {
        record.compaction_usage = previous.compaction_usage.clone();
    }
    if record.token_savings.is_none()
        && let (Some(before), Some(after)) = (
            record.pre_compaction_usage.as_ref(),
            record.post_compaction_usage.as_ref(),
        )
    {
        record.token_savings = Some(TokenSavings::from_snapshots(
            Some(before),
            Some(after),
            "hook_checkpoint",
        ));
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
) -> Result<Option<CompactCheckpoint>, JevxError> {
    latest_checkpoint_where(state_dir, session_id_sha256, |event| event == "PreCompact")
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
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(checkpoint) = serde_json::from_str::<CompactCheckpoint>(line) else {
            continue;
        };
        if checkpoint.session_id_sha256.as_deref() == Some(session_id_sha256)
            && event_matches(checkpoint.hook_event_name.as_str())
        {
            latest = Some(checkpoint);
        }
    }
    Ok(latest)
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
    payload.get("hook_event_name").and_then(Value::as_str) == Some("SessionStart")
        && payload.get("source").and_then(Value::as_str) == Some("compact")
}

fn is_post_compact(payload: &Value) -> bool {
    payload.get("hook_event_name").and_then(Value::as_str) == Some("PostCompact")
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
    }

    #[test]
    fn checkpoint_and_context_helpers_handle_missing_invalid_and_empty_state() {
        let root = tempdir().expect("tempdir");
        assert!(
            latest_checkpoint(&root.path().join("missing"), None)
                .expect("missing")
                .is_none()
        );

        let checkpoint = CompactCheckpoint {
            schema_version: COMPACT_CHECKPOINT_SCHEMA_VERSION,
            mode: "compact-assist".to_owned(),
            hook_event_name: "PreCompact".to_owned(),
            trigger: Some("manual".to_owned()),
            source: None,
            session_id_sha256: Some("session-hash".to_owned()),
            turn_id_sha256: None,
            correlation_id_sha256: None,
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
        };
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
        assert_eq!(
            latest_checkpoint(
                checkpoint_path.parent().expect("state dir"),
                Some("session-hash")
            )
            .expect("latest")
            .expect("checkpoint")
            .hook_event_name,
            "PostCompact"
        );
        assert_eq!(
            latest_pre_compaction_checkpoint(
                checkpoint_path.parent().expect("state dir"),
                Some("session-hash")
            )
            .expect("latest pre")
            .expect("pre checkpoint")
            .hook_event_name,
            "PreCompact"
        );
        assert!(
            latest_checkpoint(
                checkpoint_path.parent().expect("state dir"),
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
