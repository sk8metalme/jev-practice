//! `jevx doctor`: 設定と導入状態を診断し、次に打つコマンドを示す。

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use jevx::hook_config::{HookScope, hook_config_path, installed_handler_count};
use jevx::{Config, JevxError};
use serde_json::{Value, json};

/// doctorが読む環境。テストで差し替えられるように、プロセス環境から切り離している。
pub(super) struct DoctorEnv {
    pub(super) home: Option<PathBuf>,
    pub(super) codex_home: Option<PathBuf>,
    pub(super) cwd: PathBuf,
    pub(super) path_var: Option<OsString>,
}

impl DoctorEnv {
    fn from_process() -> Self {
        let non_empty = |key: &str| {
            env::var_os(key)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        Self {
            home: non_empty("HOME"),
            codex_home: non_empty("CODEX_HOME"),
            cwd: env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            path_var: env::var_os("PATH"),
        }
    }
}

pub(super) fn run_doctor_with_config(json: bool, config: &Config) -> Result<i32, JevxError> {
    let report = doctor_report(config, &DoctorEnv::from_process());
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", doctor_human_output(&report));
    }
    Ok(0)
}

pub(super) fn doctor_report(config: &Config, env: &DoctorEnv) -> Value {
    let api_key_configured = config
        .api_key
        .as_ref()
        .is_some_and(|key| !key.trim().is_empty());
    let jevx_on_path = env
        .path_var
        .as_ref()
        .is_some_and(|paths| env::split_paths(paths).any(|dir| dir.join("jevx").is_file()));
    let skill_installed = skill_locations(env)
        .iter()
        .any(|path| path.join("jevx/SKILL.md").is_file());
    let home = env.home.clone().unwrap_or_else(|| PathBuf::from("."));
    let user_hooks = hook_status(&hook_config_path(
        HookScope::User,
        &env.cwd,
        &home,
        env.codex_home.as_deref(),
    ));
    let project_hooks = hook_status(&hook_config_path(HookScope::Project, &env.cwd, &home, None));

    let mut next_steps = Vec::new();
    if !jevx_on_path {
        next_steps.push(
            "Add the install root to PATH: export PATH=\"$HOME/.local/bin:$PATH\"".to_owned(),
        );
    }
    if !skill_installed {
        next_steps.push(
            "Install the jevx Skill: sh jevx/scripts/setup.sh --scope user (from the repository root)"
                .to_owned(),
        );
    }
    if !api_key_configured {
        next_steps.push(
            "Set AI_GATEWAY_API_KEY to use Jev. Without it, skills list, doctor, and eval --dry-run still work"
                .to_owned(),
        );
    }
    if next_steps.is_empty() {
        next_steps.push(
            "Try it: jevx skills suggest --prompt \"PDFを結合して内容を確認したい\" --json"
                .to_owned(),
        );
        let no_hooks = |status: &Value| {
            status.get("error").is_none() && status["installedHandlers"].as_u64().unwrap_or(0) == 0
        };
        if no_hooks(&user_hooks) && no_hooks(&project_hooks) {
            next_steps.push(
                "Optional: preview Codex hooks with jevx hooks install --scope user --dry-run --json"
                    .to_owned(),
            );
        }
    }

    json!({
        "schemaVersion": 2,
        "apiKeyConfigured": api_key_configured,
        "endpoint": config.endpoint,
        "telemetryPath": config.telemetry_path,
        "telemetryEnabled": config.telemetry_enabled,
        "platform": "macos",
        "jevxOnPath": jevx_on_path,
        "skillInstalled": skill_installed,
        "hooks": {"user": user_hooks, "project": project_hooks},
        "thresholds": {
            "minProbability": config.min_probability,
            "minMargin": config.min_margin,
            "maxCandidates": config.max_candidates,
            "customized": config.thresholds_customized(),
        },
        "limits": {
            "requestTimeoutMs": u64::try_from(config.timeout.as_millis()).unwrap_or(u64::MAX),
            "maxStateBytes": config.max_state_bytes,
            "maxRetries": config.max_retries,
            "retryBackoffMs": config.retry_backoff_ms,
            "decisionCacheCapacity": config.cache_capacity,
            "inputCostWeight": config.input_cost_weight,
            "outputCostWeight": config.output_cost_weight,
            "customized": config.limits_customized(),
        },
        "pricing": {
            "currency": config.price_currency,
            "version": config.price_version,
            "jevInputPricePerMillion": config.jev_input_price_per_million,
            "jevOutputPricePerMillion": config.jev_output_price_per_million,
            "jevConfigured": config.jev_input_price_per_million.is_some()
                && config.jev_output_price_per_million.is_some(),
        },
        "warnings": config.warnings,
        "nextSteps": next_steps,
    })
}

fn skill_locations(env: &DoctorEnv) -> Vec<PathBuf> {
    let mut locations = vec![
        env.cwd.join(".agents/skills"),
        env.cwd.join(".codex/skills"),
    ];
    if let Some(home) = &env.home {
        locations.push(home.join(".agents/skills"));
        let codex_home = env
            .codex_home
            .clone()
            .unwrap_or_else(|| home.join(".codex"));
        locations.push(codex_home.join("skills"));
    }
    locations
}

fn hook_status(path: &Path) -> Value {
    match installed_handler_count(path) {
        Ok(count) => json!({"path": path, "installedHandlers": count}),
        Err(error) => json!({"path": path, "installedHandlers": null, "error": error.to_string()}),
    }
}

pub(super) fn doctor_human_output(report: &Value) -> String {
    let text = |value: &Value| match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let thresholds = &report["thresholds"];
    let mut lines = vec![
        format!("API key configured: {}", report["apiKeyConfigured"]),
        format!("Endpoint: {}", text(&report["endpoint"])),
        format!("Telemetry: {}", text(&report["telemetryPath"])),
        format!("jevx on PATH: {}", report["jevxOnPath"]),
        format!("Skill installed: {}", report["skillInstalled"]),
        format!(
            "Hooks: user={} project={}",
            report["hooks"]["user"]["installedHandlers"],
            report["hooks"]["project"]["installedHandlers"]
        ),
        format!(
            "Thresholds: minProbability={} minMargin={} maxCandidates={} customized={}",
            thresholds["minProbability"],
            thresholds["minMargin"],
            thresholds["maxCandidates"],
            thresholds["customized"]
        ),
    ];
    let limits = &report["limits"];
    lines.push(format!(
        "Limits: requestTimeoutMs={} maxStateBytes={} maxRetries={} retryBackoffMs={} decisionCacheCapacity={} customized={}",
        limits["requestTimeoutMs"],
        limits["maxStateBytes"],
        limits["maxRetries"],
        limits["retryBackoffMs"],
        limits["decisionCacheCapacity"],
        limits["customized"]
    ));
    let pricing = &report["pricing"];
    lines.push(format!(
        "Pricing: currency={} version={} jevConfigured={} inputPerMillion={} outputPerMillion={}",
        text(&pricing["currency"]),
        text(&pricing["version"]),
        pricing["jevConfigured"],
        text(&pricing["jevInputPricePerMillion"]),
        text(&pricing["jevOutputPricePerMillion"]),
    ));
    for warning in report["warnings"].as_array().into_iter().flatten() {
        lines.push(format!("Warning: {}", text(warning)));
    }
    lines.push("Next steps:".to_owned());
    for step in report["nextSteps"].as_array().into_iter().flatten() {
        lines.push(format!("  - {}", text(step)));
    }
    lines.join("\n") + "\n"
}
