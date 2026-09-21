use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Value, json};

use crate::JevxError;

const SHADOW_MARKER: &str = " hooks shadow";
const COMPACT_ASSIST_MARKER: &str = " hooks compact-assist";

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
    );
    let config = merge_hook_config(existing.as_ref(), &generated)?;
    let changed = existing
        .as_ref()
        .map(|value| value != &config)
        .unwrap_or(true);
    let mut backup_path = None;

    if changed && !options.dry_run {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if path.exists() {
            let backup = path.with_file_name(format!(
                "{}.jevx.bak",
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("hooks.json")
            ));
            if !backup.exists() {
                fs::copy(&path, &backup)?;
                backup_path = Some(backup);
            }
        }
        let mut content = serde_json::to_vec_pretty(&config)?;
        content.push(b'\n');
        fs::write(&path, content)?;
    }

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

fn generated_config(executable: &Path, records_path: &Path, state_dir: &Path) -> Value {
    let shadow_command = format!(
        "{} hooks shadow --output {}",
        shell_quote(executable),
        shell_quote(records_path)
    );
    let compact_assist_command = format!(
        "{} hooks compact-assist --state-dir {}",
        shell_quote(executable),
        shell_quote(state_dir)
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
                    &compact_assist_command,
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
    handler.get("type").and_then(Value::as_str) == Some("command")
        && handler
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(|command| {
                command.contains(SHADOW_MARKER) || command.contains(COMPACT_ASSIST_MARKER)
            })
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
        );
        let merged = merge_hook_config(Some(&existing), &generated).expect("merge");
        let groups = merged["hooks"]["UserPromptSubmit"]
            .as_array()
            .expect("groups");
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0]["hooks"][0]["command"], "custom");
        assert!(
            groups[1]["hooks"][0]["command"]
                .as_str()
                .expect("command")
                .contains("/new/jevx")
        );
    }

    #[test]
    fn merge_rejects_invalid_hooks_shape() {
        let generated = generated_config(
            Path::new("/jevx"),
            Path::new("/events"),
            Path::new("/state"),
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
}
