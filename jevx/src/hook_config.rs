use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Value, json};

use crate::JevxError;

const JEVX_BINARY_NAME: &str = "jevx";
const SHADOW_SUBCOMMAND: &str = "shadow";
const REVIEW_SUBCOMMAND: &str = "review";
const COMPACT_ASSIST_SUBCOMMAND: &str = "compact-assist";
const JEVX_MANAGED_FLAG: &str = "--jevx-managed";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookScope {
    User,
    Project,
}

#[derive(Debug, Clone)]
pub struct HookInstallOptions {
    pub scope: HookScope,
    pub repo: PathBuf,
    pub home: PathBuf,
    pub codex_home: Option<PathBuf>,
    pub executable: PathBuf,
    pub records_path: PathBuf,
    pub state_dir: PathBuf,
    pub allow_review_content: bool,
    pub allow_compact_context: bool,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HookInstallReport {
    pub path: PathBuf,
    pub changed: bool,
    #[serde(rename = "dryRun")]
    pub dry_run: bool,
    #[serde(rename = "recordsPath")]
    pub records_path: PathBuf,
    #[serde(rename = "stateDir")]
    pub state_dir: PathBuf,
    #[serde(rename = "backupPath", skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<PathBuf>,
    pub config: Value,
}

#[derive(Debug, Clone)]
pub struct HookUninstallOptions {
    pub scope: HookScope,
    pub repo: PathBuf,
    pub home: PathBuf,
    pub codex_home: Option<PathBuf>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HookUninstallReport {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    pub path: PathBuf,
    pub exists: bool,
    pub changed: bool,
    #[serde(rename = "dryRun")]
    pub dry_run: bool,
    #[serde(rename = "removedHandlers")]
    pub removed_handlers: usize,
    pub config: Option<Value>,
}

pub fn hook_config_path(
    scope: HookScope,
    repo: &Path,
    home: &Path,
    codex_home: Option<&Path>,
) -> PathBuf {
    match scope {
        HookScope::Project => repo.join(".codex/hooks.json"),
        HookScope::User => codex_home
            .map(Path::to_path_buf)
            .unwrap_or_else(|| home.join(".codex"))
            .join("hooks.json"),
    }
}

pub fn install_hooks(options: &HookInstallOptions) -> Result<HookInstallReport, JevxError> {
    let path = hook_config_path(
        options.scope,
        &options.repo,
        &options.home,
        options.codex_home.as_deref(),
    );
    let existing = if path.exists() {
        let content = fs::read_to_string(&path)?;
        Some(serde_json::from_str::<Value>(&content)?)
    } else {
        None
    };
    let generated = generated_config(
        &options.executable,
        &options.records_path,
        &options.state_dir,
        options.allow_review_content,
        options.allow_compact_context,
    );
    let config = merge_hook_config(existing.as_ref(), &generated)?;
    let changed = existing
        .as_ref()
        .map(|value| value != &config)
        .unwrap_or(true);
    let backup_path = if changed && !options.dry_run {
        write_with_backup(&path, &config)?
    } else {
        None
    };

    Ok(HookInstallReport {
        path,
        changed,
        dry_run: options.dry_run,
        records_path: options.records_path.clone(),
        state_dir: options.state_dir.clone(),
        backup_path,
        config,
    })
}

/// Removes only the handlers that `install_hooks` manages and keeps every other hook.
/// Groups and events that become empty because of the removal are dropped.
pub fn uninstall_hooks(options: &HookUninstallOptions) -> Result<HookUninstallReport, JevxError> {
    let path = hook_config_path(
        options.scope,
        &options.repo,
        &options.home,
        options.codex_home.as_deref(),
    );
    let mut report = HookUninstallReport {
        schema_version: 1,
        path,
        exists: false,
        changed: false,
        dry_run: options.dry_run,
        removed_handlers: 0,
        config: None,
    };
    if !report.path.exists() {
        return Ok(report);
    }
    report.exists = true;
    let mut config = serde_json::from_str::<Value>(&fs::read_to_string(&report.path)?)?;
    report.removed_handlers = strip_jevx_handlers(&mut config)?;
    report.changed = report.removed_handlers > 0;
    // Uninstall only removes jevx handlers, so it writes without a backup: a backup taken here
    // would contain jevx hooks and restoring it would bring them back.
    if report.changed && !options.dry_run {
        write_config(&report.path, &config)?;
    }
    report.config = Some(config);
    Ok(report)
}

/// `hooks.json` に登録済みのjevx handler数。ファイルがなければ `None`。
pub fn installed_handler_count(path: &Path) -> Result<Option<usize>, JevxError> {
    if !path.exists() {
        return Ok(None);
    }
    let mut config = serde_json::from_str::<Value>(&fs::read_to_string(path)?)?;
    strip_jevx_handlers(&mut config).map(Some)
}

fn strip_jevx_handlers(config: &mut Value) -> Result<usize, JevxError> {
    let root = config
        .as_object_mut()
        .ok_or_else(|| JevxError::InvalidInput("hooks.json root must be an object".to_owned()))?;
    let Some(hooks) = root.get_mut("hooks") else {
        return Ok(0);
    };
    let hooks = hooks
        .as_object_mut()
        .ok_or_else(|| JevxError::InvalidInput("hooks must be an object".to_owned()))?;
    let mut removed = 0;
    let mut emptied_events = Vec::new();
    for (event, groups) in hooks.iter_mut() {
        // jevxが書かない形のeventは、壊さずにそのまま残す。
        let Some(groups) = groups.as_array_mut() else {
            continue;
        };
        let mut removed_in_event = 0;
        groups.retain_mut(|group| {
            let count = count_jevx_handlers(group);
            removed_in_event += count;
            !(count > 0 && remove_jevx_handlers(group))
        });
        removed += removed_in_event;
        if removed_in_event > 0 && groups.is_empty() {
            emptied_events.push(event.clone());
        }
    }
    for event in emptied_events {
        hooks.remove(&event);
    }
    Ok(removed)
}

fn count_jevx_handlers(group: &Value) -> usize {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .map(|handlers| {
            handlers
                .iter()
                .filter(|handler| is_jevx_handler(handler))
                .count()
        })
        .unwrap_or(0)
}

/// Writes the config and keeps the first copy before jevx changed it as `<name>.jevx.bak`.
fn write_with_backup(path: &Path, config: &Value) -> Result<Option<PathBuf>, JevxError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut backup_path = None;
    if path.exists() {
        let backup = path.with_file_name(format!(
            "{}.jevx.bak",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("hooks.json")
        ));
        if !backup.exists() {
            fs::copy(path, &backup)?;
            backup_path = Some(backup);
        }
    }
    write_config(path, config)?;
    Ok(backup_path)
}

fn write_config(path: &Path, config: &Value) -> Result<(), JevxError> {
    let mut content = serde_json::to_vec_pretty(config)?;
    content.push(b'\n');
    fs::write(path, content)?;
    Ok(())
}

fn generated_config(
    executable: &Path,
    records_path: &Path,
    state_dir: &Path,
    allow_review_content: bool,
    allow_compact_context: bool,
) -> Value {
    let shadow_command = format!(
        "{} hooks shadow --output {} {JEVX_MANAGED_FLAG}",
        shell_quote(executable),
        shell_quote(records_path)
    );
    let compact_assist_command = format!(
        "{} hooks compact-assist --state-dir {} {JEVX_MANAGED_FLAG}",
        shell_quote(executable),
        shell_quote(state_dir)
    );
    let pre_compact_command = if allow_compact_context {
        format!("{} --allow-compact-context", compact_assist_command)
    } else {
        compact_assist_command.clone()
    };
    let review_content_flag = if allow_review_content {
        " --allow-content"
    } else {
        ""
    };
    let review_prompt_command = format!(
        "{} hooks review --target prompt{review_content_flag} {JEVX_MANAGED_FLAG}",
        shell_quote(executable),
    );
    let review_final_command = format!(
        "{} hooks review --target final-answer{review_content_flag} {JEVX_MANAGED_FLAG}",
        shell_quote(executable),
    );
    let review_diff_command = format!(
        "{} hooks review --target diff{review_content_flag} {JEVX_MANAGED_FLAG}",
        shell_quote(executable),
    );
    json!({
        "hooks": {
            "SessionStart": [{
                "matcher": "startup|resume|clear|compact",
                "hooks": [command_handler(
                    &compact_assist_command,
                    "jevx: restoring compact checkpoint",
                )]
            }],
            "PreCompact": [{
                "matcher": "manual|auto",
                "hooks": [command_handler(
                    &pre_compact_command,
                    "jevx: recording compact checkpoint",
                )]
            }],
            "PostCompact": [{
                "matcher": "manual|auto",
                "hooks": [command_handler(
                    &compact_assist_command,
                    "jevx: recording compact completion",
                )]
            }],
            "UserPromptSubmit": [{
                "hooks": [command_handler(
                    &shadow_command,
                    "jevx: measuring Skill suggestion",
                )]
            }, {
                "hooks": [command_handler(
                    &review_prompt_command,
                    "jevx: reviewing prompt semantics",
                )]
            }],
            "PostToolUse": [{
                "hooks": [command_handler(
                    &review_diff_command,
                    "jevx: reviewing changed-code semantics",
                )]
            }],
            "Stop": [{
                "hooks": [command_handler(
                    &review_final_command,
                    "jevx: reviewing final answer semantics",
                )]
            }]
        }
    })
}

fn command_handler(command: &str, status_message: &str) -> Value {
    json!({
        "type": "command",
        "command": command,
        "statusMessage": status_message,
        "timeout": 5
    })
}

fn merge_hook_config(existing: Option<&Value>, generated: &Value) -> Result<Value, JevxError> {
    let mut root = existing.cloned().unwrap_or_else(|| json!({}));
    let root_object = root
        .as_object_mut()
        .ok_or_else(|| JevxError::InvalidInput("hooks.json root must be an object".to_owned()))?;
    let generated_hooks = generated
        .get("hooks")
        .and_then(Value::as_object)
        .ok_or_else(|| JevxError::InvalidInput("generated hook config is invalid".to_owned()))?;
    let hooks_value = root_object
        .entry("hooks".to_owned())
        .or_insert_with(|| json!({}));
    let hooks = hooks_value
        .as_object_mut()
        .ok_or_else(|| JevxError::InvalidInput("hooks must be an object".to_owned()))?;

    for (event, generated_groups) in generated_hooks {
        let groups = hooks
            .entry(event.clone())
            .or_insert_with(|| Value::Array(Vec::new()));
        let groups = groups
            .as_array_mut()
            .ok_or_else(|| JevxError::InvalidInput(format!("hooks.{event} must be an array")))?;
        groups.retain_mut(|group| !remove_jevx_handlers(group));
        let generated_groups = generated_groups.as_array().ok_or_else(|| {
            JevxError::InvalidInput("generated hook groups are invalid".to_owned())
        })?;
        groups.extend(generated_groups.iter().cloned());
    }
    Ok(root)
}

fn remove_jevx_handlers(group: &mut Value) -> bool {
    let Some(group) = group.as_object_mut() else {
        return false;
    };
    let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
        return false;
    };
    handlers.retain(|handler| !is_jevx_handler(handler));
    handlers.is_empty()
}

fn is_jevx_handler(handler: &Value) -> bool {
    let Some(command) = handler.get("command").and_then(Value::as_str) else {
        return false;
    };
    if handler.get("type").and_then(Value::as_str) != Some("command") {
        return false;
    }
    let Some((executable, arguments)) = command.split_once(" hooks ") else {
        return false;
    };
    let executable = executable.trim().trim_matches('\'');
    let is_jevx_binary = Path::new(executable)
        .file_name()
        .and_then(|name| name.to_str())
        == Some(JEVX_BINARY_NAME);
    let is_managed = arguments
        .split_whitespace()
        .any(|argument| argument == JEVX_MANAGED_FLAG);
    (is_jevx_binary || is_managed)
        && (arguments == SHADOW_SUBCOMMAND
            || arguments.starts_with(&format!("{SHADOW_SUBCOMMAND} "))
            || arguments == REVIEW_SUBCOMMAND
            || arguments.starts_with(&format!("{REVIEW_SUBCOMMAND} "))
            || arguments == COMPACT_ASSIST_SUBCOMMAND
            || arguments.starts_with(&format!("{COMPACT_ASSIST_SUBCOMMAND} ")))
}

fn shell_quote(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_path_prefers_codex_home_and_project_path_is_repo_relative() {
        assert_eq!(
            hook_config_path(
                HookScope::User,
                Path::new("/repo"),
                Path::new("/home/user"),
                Some(Path::new("/custom/codex")),
            ),
            PathBuf::from("/custom/codex/hooks.json")
        );
        assert_eq!(
            hook_config_path(
                HookScope::User,
                Path::new("/repo"),
                Path::new("/home/user"),
                None,
            ),
            PathBuf::from("/home/user/.codex/hooks.json")
        );
        assert_eq!(
            hook_config_path(
                HookScope::Project,
                Path::new("/repo"),
                Path::new("/home/user"),
                None,
            ),
            PathBuf::from("/repo/.codex/hooks.json")
        );
    }

    #[test]
    fn merge_keeps_custom_handlers_and_replaces_jevx_handlers() {
        let existing = json!({
            "hooks": {
                "UserPromptSubmit": [{
                    "hooks": [
                        {"type":"command","command":"custom"},
                        {"type":"command","command":"/old/jevx hooks shadow --output /old/log"}
                    ]
                }]
            }
        });
        let generated = generated_config(
            Path::new("/new/jevx"),
            Path::new("/events"),
            Path::new("/state"),
            false,
            false,
        );
        let merged = merge_hook_config(Some(&existing), &generated).expect("merge");
        let groups = merged["hooks"]["UserPromptSubmit"]
            .as_array()
            .expect("groups");
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0]["hooks"][0]["command"], "custom");
        assert!(
            groups[1]["hooks"][0]["command"]
                .as_str()
                .expect("command")
                .contains("/new/jevx")
        );
    }

    #[test]
    fn compact_content_opt_in_is_independent_and_off_by_default() {
        let default = generated_config(
            Path::new("/jevx"),
            Path::new("/events"),
            Path::new("/state"),
            false,
            false,
        );
        let compact_command = default["hooks"]["PreCompact"][0]["hooks"][0]["command"]
            .as_str()
            .expect("compact command");
        assert!(!compact_command.contains("--allow-compact-context"));

        let opted_in = generated_config(
            Path::new("/jevx"),
            Path::new("/events"),
            Path::new("/state"),
            false,
            true,
        );
        let pre_compact = opted_in["hooks"]["PreCompact"][0]["hooks"][0]["command"]
            .as_str()
            .expect("pre compact command");
        assert!(pre_compact.contains("--allow-compact-context"));
        assert!(!pre_compact.contains("--allow-content"));
        for event in ["SessionStart", "PostCompact"] {
            let command = opted_in["hooks"][event][0]["hooks"][0]["command"]
                .as_str()
                .expect("compact command");
            assert!(!command.contains("--allow-compact-context"));
        }
        let review_command = opted_in["hooks"]["UserPromptSubmit"][1]["hooks"][0]["command"]
            .as_str()
            .expect("review command");
        assert!(!review_command.contains("--allow-compact-context"));
        assert!(!review_command.contains("--allow-content"));
    }

    #[test]
    fn merge_rejects_invalid_hooks_shape() {
        let generated = generated_config(
            Path::new("/jevx"),
            Path::new("/events"),
            Path::new("/state"),
            false,
            false,
        );
        let invalid = json!({"hooks": []});
        let error = merge_hook_config(Some(&invalid), &generated).expect_err("invalid hooks");
        assert!(error.to_string().contains("hooks must be an object"));

        let invalid_event = json!({"hooks":{"UserPromptSubmit":{}}});
        let error = merge_hook_config(Some(&invalid_event), &generated).expect_err("invalid event");
        assert!(error.to_string().contains("must be an array"));

        let invalid_generated = json!({"hooks":{"UserPromptSubmit":{}}});
        let error = merge_hook_config(None, &invalid_generated).expect_err("invalid generated");
        assert!(error.to_string().contains("generated hook groups"));
    }

    #[test]
    fn install_creates_parent_and_preserves_existing_backup() {
        let root = tempfile::tempdir().expect("tempdir");
        let options = HookInstallOptions {
            scope: HookScope::Project,
            repo: root.path().to_path_buf(),
            home: root.path().join("home"),
            codex_home: None,
            executable: PathBuf::from("/bin/jevx"),
            records_path: root.path().join("data/hooks.jsonl"),
            state_dir: root.path().join("data/compaction"),
            allow_review_content: false,
            allow_compact_context: false,
            dry_run: false,
        };
        let report = install_hooks(&options).expect("install");
        assert!(report.path.exists());
        assert!(report.backup_path.is_none());

        let backup = report.path.with_file_name("hooks.json.jevx.bak");
        fs::write(&backup, "existing backup").expect("backup");
        fs::write(&report.path, r#"{"hooks":{}}"#).expect("existing config");
        let changed = install_hooks(&options).expect("reinstall");
        assert!(changed.changed);
        assert!(changed.backup_path.is_none());
        assert_eq!(
            fs::read_to_string(backup).expect("backup contents"),
            "existing backup"
        );

        let mut not_a_group = json!("not a group");
        assert!(!remove_jevx_handlers(&mut not_a_group));
        let mut no_hooks = json!({"matcher":"startup"});
        assert!(!remove_jevx_handlers(&mut no_hooks));
    }

    fn uninstall_options(root: &Path, dry_run: bool) -> HookUninstallOptions {
        HookUninstallOptions {
            scope: HookScope::Project,
            repo: root.to_path_buf(),
            home: root.join("home"),
            codex_home: None,
            dry_run,
        }
    }

    #[test]
    fn uninstall_removes_only_jevx_handlers_and_keeps_custom_hooks() {
        let root = tempfile::tempdir().expect("tempdir");
        let path = root.path().join(".codex/hooks.json");
        fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        let existing = json!({
            "custom": true,
            "hooks": {
                "UserPromptSubmit": [
                    {"hooks": [
                        {"type":"command","command":"custom"},
                        {"type":"command","command":"'/bin/jevx' hooks shadow --output '/log' --jevx-managed"}
                    ]}
                ],
                "PreCompact": [
                    {"matcher":"manual|auto","hooks": [
                        {"type":"command","command":"'/bin/jevx' hooks compact-assist --state-dir '/s' --jevx-managed"}
                    ]}
                ],
                "Stop": [{"hooks": [{"type":"command","command":"notify"}]}]
            }
        });
        fs::write(&path, serde_json::to_vec_pretty(&existing).expect("json")).expect("write");

        let preview = uninstall_hooks(&uninstall_options(root.path(), true)).expect("dry run");
        assert!(preview.dry_run);
        assert!(preview.changed);
        assert_eq!(preview.removed_handlers, 2);
        let untouched: Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("read")).expect("parse");
        assert_eq!(untouched, existing, "dry run must not write");

        let report = uninstall_hooks(&uninstall_options(root.path(), false)).expect("uninstall");
        assert!(report.changed);
        assert_eq!(report.removed_handlers, 2);
        assert!(!path.with_file_name("hooks.json.jevx.bak").exists());
        let written: Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("read")).expect("parse");
        assert_eq!(written["custom"], true);
        assert_eq!(
            written["hooks"]["UserPromptSubmit"],
            json!([{"hooks": [{"type":"command","command":"custom"}]}])
        );
        assert!(written["hooks"].get("PreCompact").is_none());
        assert_eq!(written["hooks"]["Stop"], existing["hooks"]["Stop"]);

        let again = uninstall_hooks(&uninstall_options(root.path(), false)).expect("idempotent");
        assert!(!again.changed);
        assert_eq!(again.removed_handlers, 0);
    }

    #[test]
    fn uninstall_round_trips_install_and_handles_missing_or_invalid_config() {
        let root = tempfile::tempdir().expect("tempdir");
        let missing = uninstall_hooks(&uninstall_options(root.path(), false)).expect("missing");
        assert!(!missing.exists);
        assert!(!missing.changed);
        assert!(missing.config.is_none());

        install_hooks(&HookInstallOptions {
            scope: HookScope::Project,
            repo: root.path().to_path_buf(),
            home: root.path().join("home"),
            codex_home: None,
            executable: PathBuf::from("/bin/jevx"),
            records_path: root.path().join("data/hooks.jsonl"),
            state_dir: root.path().join("data/compaction"),
            allow_review_content: false,
            allow_compact_context: false,
            dry_run: false,
        })
        .expect("install");
        let report = uninstall_hooks(&uninstall_options(root.path(), false)).expect("uninstall");
        assert!(report.exists);
        assert_eq!(report.removed_handlers, 7);
        assert_eq!(report.config, Some(json!({"hooks": {}})));

        fs::write(&report.path, r#"{"hooks": []}"#).expect("invalid");
        let error = uninstall_hooks(&uninstall_options(root.path(), false)).expect_err("invalid");
        assert!(error.to_string().contains("hooks must be an object"));
        fs::write(&report.path, r#"[]"#).expect("invalid root");
        let error = uninstall_hooks(&uninstall_options(root.path(), false)).expect_err("root");
        assert!(error.to_string().contains("root must be an object"));
        fs::write(
            &report.path,
            r#"{"hooks": {"Stop": {}, "UserPromptSubmit": [{"hooks": [{"type":"command","command":"/bin/jevx hooks shadow --jevx-managed"}]}]}}"#,
        )
        .expect("foreign event shape");
        let skipped = uninstall_hooks(&uninstall_options(root.path(), false))
            .expect("events that are not arrays are left untouched");
        assert_eq!(skipped.removed_handlers, 1);
        assert_eq!(skipped.config.expect("config")["hooks"]["Stop"], json!({}));
        fs::write(&report.path, r#"{"other": 1}"#).expect("no hooks");
        let none = uninstall_hooks(&uninstall_options(root.path(), false)).expect("no hooks");
        assert!(!none.changed);
    }
}
