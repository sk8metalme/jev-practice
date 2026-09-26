use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Config;
use crate::JevxError;
use crate::cost::{CostEstimate, total_cost};
use crate::decision::{
    DecisionContract, DecisionExecution, DecisionExecutionOptions, DecisionJudge, QuestionSpec,
    StatePlan, execute_contract,
};
use crate::hooks::{
    HookShadowRecord, TokenSavings, TokenUsageSnapshot, append_shadow_record, hook_event_name,
    run_shadow,
};
use crate::recorder::{DecisionReceipt, DecisionRecorder, JsonlDecisionRecorder};
use crate::redaction::{redact, sha256_hex};
use crate::storage::append_json_line;

const CONTEXT_LIMIT: usize = 4_000;
const CONTEXT_READ_LIMIT_BYTES: u64 = CONTEXT_LIMIT as u64 * 4 + 1;
const CONTEXT_TRUNCATION_MARKER: &str = "\n[jevx context truncated]";
pub(crate) const COMPACT_CHECKPOINT_SCHEMA_VERSION: u8 = 5;
pub(crate) const COMPACT_ASSIST_LOCK_FILE_NAME: &str = ".compact-assist.lock";

#[derive(Debug, Clone, Serialize)]
pub struct CompactAssistResult {
    pub response: Value,
    pub checkpoint: CompactCheckpoint,
    #[serde(skip)]
    pub warning_code: Option<String>,
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
    #[serde(
        rename = "decisionReplayId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub decision_replay_id: Option<String>,
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
    truncated: bool,
    redaction_applied: bool,
}

pub async fn run_compact_assist(
    input: &str,
    config: &Config,
    state_dir: &Path,
) -> Result<CompactAssistResult, JevxError> {
    run_compact_assist_with_judge(input, config, state_dir, false, None).await
}

/// `allow_compact_context` permits sending only the locally redacted, bounded
/// `.jevx/compact-context.md` manifest for an advisory PreCompact decision.
pub async fn run_compact_assist_with_opt_in(
    input: &str,
    config: &Config,
    state_dir: &Path,
    allow_compact_context: bool,
) -> Result<CompactAssistResult, JevxError> {
    let judge = allow_compact_context
        .then(|| crate::GatewayJudge::from_config(config).ok())
        .flatten();
    run_compact_assist_with_judge(
        input,
        config,
        state_dir,
        allow_compact_context,
        judge.as_ref().map(|judge| judge as &dyn DecisionJudge),
    )
    .await
}

async fn run_compact_assist_with_judge(
    input: &str,
    config: &Config,
    state_dir: &Path,
    allow_compact_context: bool,
    judge: Option<&dyn DecisionJudge>,
) -> Result<CompactAssistResult, JevxError> {
    let started = Instant::now();
    let payload: Value = serde_json::from_str(input)?;
    let mut shadow = run_shadow(input, None, &[], config, None).await?;
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut warning_code = None;
    let context = match read_context(&cwd) {
        Ok(context) => context,
        Err(JevxError::Io(_)) => {
            warning_code = Some("compact_context_read_failed".to_owned());
            shadow.record.error_code = warning_code.clone();
            None
        }
        Err(error) => return Err(error),
    };
    let decision_replay_id =
        if allow_compact_context && hook_event_name(&payload) == Some("PreCompact") {
            if let Some(context) = context.as_ref() {
                let contract = compact_context_contract(config);
                match compact_context_state(context, config) {
                    Ok(state) => {
                        let execution_started = Instant::now();
                        let execution = execute_contract(
                            &contract,
                            &state,
                            judge,
                            &DecisionExecutionOptions::default(),
                        )
                        .await;
                        apply_compact_decision_metrics(
                            &mut shadow.record,
                            &execution,
                            config,
                            elapsed_millis(execution_started),
                        );
                        match persist_compact_decision(config, &contract, &state, &execution) {
                            Ok(replay_id) => replay_id,
                            Err(_) => {
                                let code = "decision_receipt_write_failed".to_owned();
                                shadow.record.error_code = Some(code.clone());
                                warning_code.get_or_insert(code);
                                None
                            }
                        }
                    }
                    Err(_) => {
                        let code = "compact_decision_state_failed".to_owned();
                        shadow.record.error_code = Some(code.clone());
                        warning_code.get_or_insert(code);
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };
    let persistence = (|| {
        fs::create_dir_all(state_dir)?;
        with_compaction_lock(state_dir, || {
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
            let checkpoint_replay_id = previous_pre_checkpoint
                .as_ref()
                .and_then(|previous| previous.decision_replay_id.clone())
                .or_else(|| decision_replay_id.clone());
            let checkpoint = checkpoint_from(
                &payload,
                &shadow.record,
                &cwd,
                context.as_ref(),
                checkpoint_replay_id,
            );

            shadow.record.elapsed_ms = elapsed_millis(started);
            append_shadow_record(&state_dir.join("hook-records.jsonl"), &shadow.record)?;
            append_checkpoint(&state_dir.join("checkpoints.jsonl"), &checkpoint)?;
            Ok((prior_checkpoint, checkpoint))
        })
    })();
    let (prior_checkpoint, checkpoint) = match persistence {
        Ok(result) => result,
        Err(JevxError::Io(_)) => {
            let code = "compact_assist_storage_failed".to_owned();
            shadow.record.error_code = Some(code.clone());
            shadow.record.elapsed_ms = elapsed_millis(started);
            warning_code.get_or_insert(code);
            (
                None,
                checkpoint_from(&payload, &shadow.record, &cwd, context.as_ref(), None),
            )
        }
        Err(error) => return Err(error),
    };

    let response = if is_compact_session_start(&payload) {
        session_start_response(prior_checkpoint.as_ref(), context.as_ref())
    } else {
        serde_json::to_value(shadow.response)?
    };

    Ok(CompactAssistResult {
        response,
        checkpoint,
        warning_code,
    })
}

fn compact_context_contract(config: &Config) -> DecisionContract {
    let mut contract = DecisionContract::choice(
        "compact-context-retention",
        "compact-context-retention.v1",
        BTreeMap::from([
            (
                "retain".to_owned(),
                "後続作業に役立つ情報がredacted manifestに含まれています。".to_owned(),
            ),
            (
                "omit".to_owned(),
                "redacted manifestは重複しているか、後続作業に役立ちません。".to_owned(),
            ),
        ]),
        config.min_probability,
        config.min_margin,
    );
    if let QuestionSpec::Choice { instructions, .. } = &mut contract.question {
        *instructions = "次のredacted manifestを評価し、後続作業の補足contextとして残す価値があればretain、不要ならomitを選んでください。要約・書き換え・削除・Compaction動作の変更はしないでください。".to_owned();
    }
    contract
}

fn compact_context_state(
    context: &ContextSnapshot,
    config: &Config,
) -> Result<StatePlan, JevxError> {
    let mut omitted = Vec::new();
    if context.truncated {
        omitted.push("compact_context_truncated".to_owned());
    }
    let redaction_reasons = if context.redaction_applied {
        vec!["compact_context_secret_pattern".to_owned()]
    } else {
        Vec::new()
    };
    let payload = json!({
        "source": "local_compact_context",
        "manifest": context.redacted,
        "manifestChars": context.chars,
        "manifestTruncated": context.truncated,
    });
    Ok(
        StatePlan::from_value(payload, 2, 1, omitted, redaction_reasons)?
            .with_budgets(config.max_state_bytes, 2),
    )
}

fn apply_compact_decision_metrics(
    record: &mut HookShadowRecord,
    execution: &DecisionExecution,
    config: &Config,
    total_ms: u64,
) {
    record.jev_response_ms = Some(execution.response_ms);
    record.total_ms = Some(total_ms);
    record.input_tokens = execution.usage.as_ref().map(|usage| usage.input_tokens);
    record.output_tokens = execution.usage.as_ref().map(|usage| usage.output_tokens);
    record.error_code = execution.error_code.clone();
    record.cost.jev = if execution.calls == 0 {
        CostEstimate::actual(0.0, None, None)
    } else {
        execution
            .usage
            .as_ref()
            .and_then(|usage| usage.cost.as_ref().filter(|cost| cost.is_valid()).cloned())
            .unwrap_or_else(|| {
                config.jev_pricing().estimate_two_part(
                    execution.usage.as_ref().map(|usage| usage.input_tokens),
                    execution.usage.as_ref().map(|usage| usage.output_tokens),
                    execution.cache_hit,
                )
            })
    };
    record.cost.total = total_cost(&record.cost.jev, &record.cost.codex);
}

fn persist_compact_decision(
    config: &Config,
    contract: &DecisionContract,
    state: &StatePlan,
    execution: &DecisionExecution,
) -> Result<Option<String>, JevxError> {
    if !config.telemetry_enabled {
        return Ok(None);
    }
    let mut receipt = DecisionReceipt::from_execution_with_pricing(
        contract,
        state,
        execution,
        config.input_cost_weight,
        config.output_cost_weight,
        &config.jev_pricing(),
    );
    if execution.calls == 0 {
        receipt.cost.jev = CostEstimate::actual(0.0, None, None);
        receipt.cost.total = total_cost(&receipt.cost.jev, &receipt.cost.codex);
    }
    JsonlDecisionRecorder::new(config.decision_receipt_path.clone()).record(&receipt)?;
    Ok(Some(receipt.replay_id))
}

fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
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
    decision_replay_id: Option<String>,
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
        decision_replay_id,
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
        1 | 2 | 3 | 4 | COMPACT_CHECKPOINT_SCHEMA_VERSION
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
    let cwd_directory = match open_context_directory(cwd) {
        Ok(directory) => directory,
        Err(error) if is_missing_or_linked_context_entry(&error) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let context_directory = match open_context_subdirectory(&cwd_directory, OsStr::new(".jevx")) {
        Ok(directory) => directory,
        Err(error) if is_missing_or_linked_context_entry(&error) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let file = match open_context_manifest(&context_directory) {
        Ok(file) => file,
        Err(error) if is_missing_or_linked_context_entry(&error) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    if !file.metadata()?.is_file() {
        return Ok(None);
    }
    file.take(CONTEXT_READ_LIMIT_BYTES)
        .read_to_end(&mut bytes)?;
    let raw = String::from_utf8_lossy(&bytes);
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let sanitized = redact(trimmed);
    let redaction_applied = sanitized != trimmed;
    let truncated =
        sanitized.chars().count() > CONTEXT_LIMIT || bytes.len() as u64 >= CONTEXT_READ_LIMIT_BYTES;
    let redacted = truncate(&sanitized, CONTEXT_LIMIT);
    Ok(Some(ContextSnapshot {
        hash: sha256_hex(&redacted),
        chars: redacted.chars().count(),
        redacted,
        truncated,
        redaction_applied,
    }))
}

fn is_missing_or_linked_context_entry(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound
        || matches!(error.raw_os_error(), Some(libc::ELOOP | libc::ENOTDIR))
}

fn open_context_directory(path: &Path) -> io::Result<File> {
    let flags = libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
    let mut directory =
        OpenOptions::new()
            .read(true)
            .custom_flags(flags)
            .open(if path.is_absolute() {
                Path::new("/")
            } else {
                Path::new(".")
            })?;

    for component in path.components() {
        directory = match component {
            Component::Prefix(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "unsupported compact context path prefix",
                ));
            }
            Component::RootDir | Component::CurDir => directory,
            Component::ParentDir => open_context_subdirectory(&directory, OsStr::new(".."))?,
            Component::Normal(name) => open_context_subdirectory(&directory, name)?,
        };
    }
    Ok(directory)
}

fn open_context_subdirectory(directory: &File, name: &OsStr) -> io::Result<File> {
    open_context_entry(
        directory,
        name,
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
    )
}

fn open_context_manifest(directory: &File) -> io::Result<File> {
    open_context_entry(
        directory,
        OsStr::new("compact-context.md"),
        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
    )
}

fn open_context_entry(directory: &File, name: &OsStr, flags: i32) -> io::Result<File> {
    let name = std::ffi::CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid compact context path"))?;
    // SAFETY: `directory` owns a valid directory fd, `name` is NUL-terminated, and a
    // successful openat fd is transferred exactly once into `File` below.
    let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` came from successful openat and ownership has not been transferred.
    Ok(unsafe { File::from_raw_fd(fd) })
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
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    if max_chars == 0 {
        return String::new();
    }
    let marker_chars = CONTEXT_TRUNCATION_MARKER.chars().count();
    if max_chars < marker_chars {
        return "…".chars().take(max_chars).collect();
    }
    let content_chars = max_chars - marker_chars;
    let mut result = value.chars().take(content_chars).collect::<String>();
    result.push_str(CONTEXT_TRUNCATION_MARKER);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::decision::{DecisionJudge, DecisionRequest, DecisionResponse, TypedAnswer};
    use crate::types::Usage;
    use async_trait::async_trait;
    use tempfile::tempdir;

    struct FixtureJudge {
        calls: AtomicUsize,
        states: Mutex<Vec<String>>,
        fail: bool,
        reported_cost: Option<CostEstimate>,
    }

    impl FixtureJudge {
        fn new(fail: bool) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                states: Mutex::new(Vec::new()),
                fail,
                reported_cost: None,
            }
        }

        fn with_reported_cost() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                states: Mutex::new(Vec::new()),
                fail: false,
                reported_cost: Some(CostEstimate::actual(
                    0.003,
                    Some("USD".to_owned()),
                    Some("fixture-usage".to_owned()),
                )),
            }
        }
    }

    #[async_trait]
    impl DecisionJudge for FixtureJudge {
        async fn evaluate(&self, request: DecisionRequest) -> Result<DecisionResponse, JevxError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.states
                .lock()
                .expect("fixture state lock")
                .push(request.state);
            if self.fail {
                return Err(JevxError::Provider("fixture provider failure".to_owned()));
            }
            Ok(DecisionResponse {
                answers: BTreeMap::from([(
                    "compact-context-retention".to_owned(),
                    TypedAnswer::Choice {
                        choice: "retain".to_owned(),
                        probabilities: BTreeMap::from([
                            ("retain".to_owned(), 0.92),
                            ("omit".to_owned(), 0.08),
                        ]),
                        confidence: Some(0.92),
                    },
                )]),
                response_ms: 17,
                usage: Some(Usage {
                    input_tokens: 123,
                    output_tokens: 22,
                    cost: self.reported_cost.clone(),
                }),
                calls: 1,
                retries: 0,
            })
        }
    }

    fn compact_context_fixture(root: &Path) -> PathBuf {
        let repo = root.join("repo");
        fs::create_dir_all(repo.join(".jevx")).expect("context directory");
        fs::write(
            repo.join(".jevx/compact-context.md"),
            "goal: keep the release checklist\nAPI_KEY: fixture-secret-123\nnext: run tests",
        )
        .expect("context manifest");
        fs::canonicalize(repo).expect("canonical repo")
    }

    fn pre_compact_payload(repo: &Path) -> String {
        json!({
            "hook_event_name": "PreCompact",
            "trigger": "manual",
            "session_id": "fixture-session",
            "turn_id": "fixture-turn",
            "cwd": repo
        })
        .to_string()
    }

    #[test]
    fn previous_usage_is_not_merged_across_compaction_cycles() {
        let mut record: HookShadowRecord = serde_json::from_value(json!({
            "schemaVersion": crate::hooks::HOOK_SCHEMA_VERSION,
            "mode": "shadow",
            "hookEventName": "PostCompact",
            "sessionIdSha256": "session-hash",
            "turnIdSha256": "turn-hash",
            "correlationIdSha256": "cycle-hash",
            "elapsedMs": 0
        }))
        .expect("minimal hook record");
        let previous = test_checkpoint(
            COMPACT_CHECKPOINT_SCHEMA_VERSION,
            "PreCompact",
            Some("different-turn"),
        );

        merge_previous_usage(&mut record, Some(&previous));

        assert!(record.pre_compaction_usage.is_none());
        assert!(record.token_savings.is_none());
    }

    #[tokio::test]
    async fn compact_decision_is_opt_in_precompact_only_and_advisory() {
        let root = tempdir().expect("tempdir");
        let repo = compact_context_fixture(root.path());
        let state_dir = root.path().join("state");
        let mut config = Config::for_test(root.path().join("data"));
        config.telemetry_enabled = true;
        let judge = FixtureJudge::new(false);

        let default = run_compact_assist_with_judge(
            &pre_compact_payload(&repo),
            &config,
            &state_dir,
            false,
            Some(&judge),
        )
        .await
        .expect("default compact assist");
        assert_eq!(judge.calls.load(Ordering::SeqCst), 0);
        assert_eq!(default.response["continue"], true);
        assert!(default.checkpoint.decision_replay_id.is_none());

        let opted_in = run_compact_assist_with_judge(
            &pre_compact_payload(&repo),
            &config,
            &state_dir,
            true,
            Some(&judge),
        )
        .await
        .expect("opted-in compact assist");
        assert_eq!(judge.calls.load(Ordering::SeqCst), 1);
        assert_eq!(opted_in.response["continue"], true);
        assert!(opted_in.checkpoint.decision_replay_id.is_some());
        let sent_state = judge.states.lock().expect("fixture states")[0].clone();
        assert!(sent_state.contains("keep the release checklist"));
        assert!(sent_state.contains("<redacted>"));
        assert!(!sent_state.contains("fixture-secret-123"));
        let sent_value: Value = serde_json::from_str(&sent_state).expect("sent state json");
        assert_eq!(sent_value["source"], "local_compact_context");
        assert!(sent_value.get("conversation").is_none());
        assert!(sent_value.get("prompt").is_none());
        let checkpoints =
            fs::read_to_string(state_dir.join("checkpoints.jsonl")).expect("checkpoint records");
        assert!(!checkpoints.contains("keep the release checklist"));
        assert!(!checkpoints.contains("fixture-secret-123"));

        let receipt_text =
            fs::read_to_string(&config.decision_receipt_path).expect("decision receipt");
        assert!(!receipt_text.contains("keep the release checklist"));
        assert!(!receipt_text.contains("fixture-secret-123"));
        assert!(receipt_text.contains("compact-context-retention"));
        assert!(
            receipt_text.contains(
                opted_in
                    .checkpoint
                    .decision_replay_id
                    .as_ref()
                    .expect("replay id")
            )
        );
        let receipt: Value =
            serde_json::from_str(receipt_text.lines().next().expect("receipt line"))
                .expect("receipt json");
        assert_eq!(receipt["candidateCount"], 2);
        assert_eq!(receipt["candidateLimit"], 2);
        let record_values = fs::read_to_string(state_dir.join("hook-records.jsonl"))
            .expect("hook records")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("hook record json"))
            .collect::<Vec<_>>();
        let measured_record = record_values.last().expect("opt-in hook record");
        assert_eq!(measured_record["jevResponseMs"], 17);
        assert_eq!(measured_record["inputTokens"], 123);
        assert_eq!(measured_record["outputTokens"], 22);
        assert_eq!(measured_record["cost"]["jev"]["status"], "unknown");
        assert_eq!(measured_record["cost"]["jev"]["amount"], Value::Null);

        let mut post_payload: Value =
            serde_json::from_str(&pre_compact_payload(&repo)).expect("pre payload");
        post_payload["hook_event_name"] = json!("PostCompact");
        let post = run_compact_assist_with_judge(
            &post_payload.to_string(),
            &config,
            &state_dir,
            true,
            Some(&judge),
        )
        .await
        .expect("post compact assist");
        assert_eq!(judge.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            post.checkpoint.decision_replay_id,
            opted_in.checkpoint.decision_replay_id
        );
        assert_eq!(post.response["continue"], true);
    }

    #[tokio::test]
    async fn compact_decision_fails_open_and_skips_missing_or_truncated_context() {
        let root = tempdir().expect("tempdir");
        let canonical_root = fs::canonicalize(root.path()).expect("canonical temp root");
        let repo = compact_context_fixture(root.path());
        let state_dir = root.path().join("state");
        let mut config = Config::for_test(root.path().join("data"));
        config.telemetry_enabled = true;

        let missing_judge = run_compact_assist_with_judge(
            &pre_compact_payload(&repo),
            &config,
            &state_dir,
            true,
            None,
        )
        .await
        .expect("missing Jev is non-blocking");
        assert_eq!(missing_judge.response["continue"], true);
        assert!(missing_judge.checkpoint.decision_replay_id.is_some());
        let records =
            fs::read_to_string(state_dir.join("hook-records.jsonl")).expect("hook records");
        assert!(records.contains("missing_api_key"));
        assert!(records.contains("\"elapsedMs\":"));
        assert!(records.contains("\"jevResponseMs\":"));
        let missing_receipt: Value = serde_json::from_str(
            fs::read_to_string(&config.decision_receipt_path)
                .expect("missing-key receipt")
                .lines()
                .next()
                .expect("missing-key receipt line"),
        )
        .expect("missing-key receipt json");
        assert_eq!(missing_receipt["cost"]["jev"]["amount"], 0.0);
        assert_eq!(missing_receipt["cost"]["jev"]["basis"], "actual");

        let no_context_repo = canonical_root.join("empty-repo");
        fs::create_dir_all(&no_context_repo).expect("empty repo");
        let judge = FixtureJudge::new(false);
        let no_context = run_compact_assist_with_judge(
            &pre_compact_payload(&no_context_repo),
            &config,
            &root.path().join("no-context-state"),
            true,
            Some(&judge),
        )
        .await
        .expect("missing context is non-blocking");
        assert_eq!(judge.calls.load(Ordering::SeqCst), 0);
        assert_eq!(no_context.response["continue"], true);

        let symlink_repo = canonical_root.join("symlink-repo");
        fs::create_dir_all(symlink_repo.join(".jevx")).expect("symlink repo");
        let outside_manifest = canonical_root.join("outside-manifest.md");
        fs::write(&outside_manifest, "goal: external linked content").expect("outside manifest");
        std::os::unix::fs::symlink(
            &outside_manifest,
            symlink_repo.join(".jevx/compact-context.md"),
        )
        .expect("manifest symlink");
        let symlink_judge = FixtureJudge::new(false);
        let symlink_context = run_compact_assist_with_judge(
            &pre_compact_payload(&symlink_repo),
            &config,
            &root.path().join("symlink-state"),
            true,
            Some(&symlink_judge),
        )
        .await
        .expect("symlink context is ignored");
        assert_eq!(symlink_judge.calls.load(Ordering::SeqCst), 0);
        assert!(!symlink_context.checkpoint.context_available);
        assert_eq!(symlink_context.response["continue"], true);

        let linked_directory_repo = canonical_root.join("linked-directory-repo");
        fs::create_dir_all(&linked_directory_repo).expect("linked directory repo");
        let outside_jevx = canonical_root.join("outside-jevx");
        fs::create_dir_all(&outside_jevx).expect("outside jevx directory");
        fs::write(
            outside_jevx.join("compact-context.md"),
            "goal: external linked directory content",
        )
        .expect("outside directory manifest");
        std::os::unix::fs::symlink(&outside_jevx, linked_directory_repo.join(".jevx"))
            .expect("jevx directory symlink");
        let linked_directory_judge = FixtureJudge::new(false);
        let linked_directory_context = run_compact_assist_with_judge(
            &pre_compact_payload(&linked_directory_repo),
            &config,
            &root.path().join("linked-directory-state"),
            true,
            Some(&linked_directory_judge),
        )
        .await
        .expect("symlinked context directory is ignored");
        assert_eq!(linked_directory_judge.calls.load(Ordering::SeqCst), 0);
        assert!(!linked_directory_context.checkpoint.context_available);
        assert_eq!(linked_directory_context.response["continue"], true);

        let real_parent = canonical_root.join("real-parent");
        let real_cwd = real_parent.join("repo");
        fs::create_dir_all(real_cwd.join(".jevx")).expect("real cwd context directory");
        fs::write(
            real_cwd.join(".jevx/compact-context.md"),
            "goal: symlinked cwd content",
        )
        .expect("real cwd manifest");
        let linked_parent = canonical_root.join("linked-parent");
        std::os::unix::fs::symlink(&real_parent, &linked_parent).expect("cwd ancestor symlink");
        let linked_cwd = canonical_root.join("linked-cwd");
        std::os::unix::fs::symlink(&real_cwd, &linked_cwd).expect("cwd symlink");
        for (label, symlinked_cwd) in [
            ("cwd", linked_cwd),
            ("cwd ancestor", linked_parent.join("repo")),
        ] {
            let symlinked_cwd_judge = FixtureJudge::new(false);
            let symlinked_cwd_context = run_compact_assist_with_judge(
                &pre_compact_payload(&symlinked_cwd),
                &config,
                &root.path().join(format!("{label}-state")),
                true,
                Some(&symlinked_cwd_judge),
            )
            .await
            .expect("symlinked cwd is ignored");
            assert_eq!(symlinked_cwd_judge.calls.load(Ordering::SeqCst), 0);
            assert!(!symlinked_cwd_context.checkpoint.context_available);
            assert_eq!(symlinked_cwd_context.response["continue"], true);
        }

        fs::write(
            repo.join(".jevx/compact-context.md"),
            "x".repeat(CONTEXT_LIMIT + 1),
        )
        .expect("oversized context");
        let truncated = run_compact_assist_with_judge(
            &pre_compact_payload(&repo),
            &config,
            &root.path().join("truncated-state"),
            true,
            Some(&judge),
        )
        .await
        .expect("truncated context is non-blocking");
        assert_eq!(judge.calls.load(Ordering::SeqCst), 0);
        assert_eq!(truncated.response["continue"], true);
        assert!(truncated.checkpoint.decision_replay_id.is_some());
    }

    #[tokio::test]
    async fn provider_failure_is_recorded_without_changing_hook_response() {
        let root = tempdir().expect("tempdir");
        let repo = compact_context_fixture(root.path());
        let judge = FixtureJudge::new(true);
        let config = Config::for_test(root.path().join("data"));
        let result = run_compact_assist_with_judge(
            &pre_compact_payload(&repo),
            &config,
            &root.path().join("state"),
            true,
            Some(&judge),
        )
        .await
        .expect("provider failure remains advisory");
        assert_eq!(judge.calls.load(Ordering::SeqCst), 1);
        assert_eq!(result.response["continue"], true);
        assert!(result.checkpoint.decision_replay_id.is_none());
    }

    #[tokio::test]
    async fn receipt_write_failure_does_not_change_hook_response() {
        let root = tempdir().expect("tempdir");
        let repo = compact_context_fixture(root.path());
        let judge = FixtureJudge::new(false);
        let mut config = Config::for_test(root.path().join("data"));
        config.telemetry_enabled = true;
        config.decision_receipt_path = root.path().join("receipt-directory");
        fs::create_dir(&config.decision_receipt_path).expect("receipt directory");
        let state_dir = root.path().join("state");

        let result = run_compact_assist_with_judge(
            &pre_compact_payload(&repo),
            &config,
            &state_dir,
            true,
            Some(&judge),
        )
        .await
        .expect("receipt write failure remains advisory");

        assert_eq!(result.response["continue"], true);
        assert!(result.checkpoint.decision_replay_id.is_none());
        assert_eq!(
            result.warning_code.as_deref(),
            Some("decision_receipt_write_failed")
        );
        let record: Value = serde_json::from_str(
            fs::read_to_string(state_dir.join("hook-records.jsonl"))
                .expect("hook record")
                .lines()
                .next()
                .expect("hook record line"),
        )
        .expect("hook record json");
        assert_eq!(record["errorCode"], "decision_receipt_write_failed");
    }

    #[tokio::test]
    async fn compact_assist_storage_failures_are_reported_without_blocking_hook() {
        let root = tempdir().expect("tempdir");
        let canonical_root = fs::canonicalize(root.path()).expect("canonical temp root");
        let repo = compact_context_fixture(&canonical_root);
        let state_dir = root.path().join("state-file");
        fs::write(&state_dir, "not a directory").expect("state path blocker");
        let config = Config::for_test(root.path().join("data"));

        let storage_failure = run_compact_assist_with_judge(
            &pre_compact_payload(&repo),
            &config,
            &state_dir,
            false,
            None,
        )
        .await
        .expect("storage failure is advisory");
        assert_eq!(storage_failure.response["continue"], true);
        assert_eq!(
            storage_failure.warning_code.as_deref(),
            Some("compact_assist_storage_failed")
        );
        assert!(
            serde_json::to_value(&storage_failure)
                .expect("compact result JSON")
                .get("warning_code")
                .is_none()
        );
        assert!(storage_failure.checkpoint.context_available);

        let long_cwd = PathBuf::from("/").join("x".repeat(300));
        let context_failure = run_compact_assist_with_judge(
            &pre_compact_payload(&long_cwd),
            &config,
            &root.path().join("context-read-state"),
            false,
            None,
        )
        .await
        .expect("context read failure is advisory");
        assert_eq!(context_failure.response["continue"], true);
        assert_eq!(
            context_failure.warning_code.as_deref(),
            Some("compact_context_read_failed")
        );
    }

    #[tokio::test]
    async fn provider_cost_is_kept_as_reported_actual_cost() {
        let root = tempdir().expect("tempdir");
        let repo = compact_context_fixture(root.path());
        let judge = FixtureJudge::with_reported_cost();
        let mut config = Config::for_test(root.path().join("data"));
        config.telemetry_enabled = true;
        let result = run_compact_assist_with_judge(
            &pre_compact_payload(&repo),
            &config,
            &root.path().join("state"),
            true,
            Some(&judge),
        )
        .await
        .expect("actual provider cost");
        assert_eq!(result.response["continue"], true);
        let receipt: Value = serde_json::from_str(
            fs::read_to_string(&config.decision_receipt_path)
                .expect("receipt")
                .lines()
                .next()
                .expect("receipt line"),
        )
        .expect("receipt json");
        assert_eq!(receipt["cost"]["jev"]["amount"], 0.003);
        assert_eq!(receipt["cost"]["jev"]["basis"], "actual");
    }

    #[tokio::test]
    async fn clean_compact_context_has_no_redaction_reason() {
        let root = tempdir().expect("tempdir");
        let repo = root.path().join("repo");
        fs::create_dir_all(repo.join(".jevx")).expect("context directory");
        fs::write(
            repo.join(".jevx/compact-context.md"),
            "goal: run the follow-up test",
        )
        .expect("clean context");
        let repo = fs::canonicalize(repo).expect("canonical repo");
        let judge = FixtureJudge::new(false);
        let mut config = Config::for_test(root.path().join("data"));
        config.telemetry_enabled = true;
        run_compact_assist_with_judge(
            &pre_compact_payload(&repo),
            &config,
            &root.path().join("state"),
            true,
            Some(&judge),
        )
        .await
        .expect("clean context decision");
        let receipt: Value = serde_json::from_str(
            fs::read_to_string(&config.decision_receipt_path)
                .expect("receipt")
                .lines()
                .next()
                .expect("receipt line"),
        )
        .expect("receipt json");
        assert_eq!(receipt["redactionReasons"], json!([]));
    }

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
            decision_replay_id: None,
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
        assert_eq!(truncate("abcdef", 3), "…");
        assert_eq!(truncate("abcdef", 0), "");
        assert_eq!(
            truncate("abcdefghijklmnopqrstuvwxyz0123456789", 32)
                .chars()
                .count(),
            32
        );
        assert_eq!(truncate("abc", 3), "abc");
        assert!(same_optional_identity(None, None));
        assert!(!same_optional_identity(None, Some("cycle-hash")));
        assert!(!is_compact_session_start(&json!({
            "hook_event_name": "SessionStart",
            "source": "startup"
        })));
        assert!(is_post_compact(&json!({"event": "PostCompact"})));
    }

    #[test]
    fn checkpoint_and_context_helpers_handle_missing_invalid_and_empty_state() {
        let root = tempdir().expect("tempdir");
        let canonical_root = fs::canonicalize(root.path()).expect("canonical temp root");
        assert!(
            latest_pre_compaction_checkpoint(
                root.path(),
                None,
                Some("turn-hash"),
                Some("cycle-hash"),
            )
            .expect("no session identity")
            .is_none()
        );
        assert!(
            latest_pre_compaction_checkpoint(
                &root.path().join("missing-pre-state"),
                Some("session-hash"),
                Some("turn-hash"),
                Some("cycle-hash"),
            )
            .expect("missing pre-checkpoint file")
            .is_none()
        );
        let blank_state = root.path().join("blank-state");
        fs::create_dir_all(&blank_state).expect("blank state directory");
        fs::write(blank_state.join("checkpoints.jsonl"), "\n").expect("blank checkpoint line");
        assert!(
            latest_pre_compaction_checkpoint(
                &blank_state,
                Some("session-hash"),
                Some("turn-hash"),
                Some("cycle-hash"),
            )
            .expect("blank checkpoint line")
            .is_none()
        );
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
            read_context(&canonical_root)
                .expect("missing context")
                .is_none()
        );
        fs::create_dir_all(root.path().join(".jevx")).expect("context dir");
        fs::write(root.path().join(".jevx/compact-context.md"), "  \n").expect("empty context");
        assert!(
            read_context(&canonical_root)
                .expect("empty context")
                .is_none()
        );
        let manifest_path = root.path().join(".jevx/compact-context.md");
        fs::remove_file(&manifest_path).expect("remove empty manifest");
        fs::create_dir(&manifest_path).expect("manifest directory");
        assert!(
            read_context(&canonical_root)
                .expect("non-file manifest")
                .is_none()
        );

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
        for version in [1, 2, 3, 4] {
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
        let canonical_root = fs::canonicalize(root.path()).expect("canonical temp root");
        fs::create_dir_all(root.path().join(".jevx")).expect("context dir");
        let mut content = vec![b'a'; CONTEXT_READ_LIMIT_BYTES as usize + 100];
        content[CONTEXT_READ_LIMIT_BYTES as usize] = 0xff;
        fs::write(root.path().join(".jevx/compact-context.md"), content).expect("context");

        let snapshot = read_context(&canonical_root)
            .expect("bounded context read")
            .expect("context snapshot");
        assert!(snapshot.redacted.contains("[jevx context truncated]"));
        assert!(snapshot.chars <= CONTEXT_LIMIT);
        assert!(snapshot.redacted.chars().count() < CONTEXT_READ_LIMIT_BYTES as usize);
    }
}
