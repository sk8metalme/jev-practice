//! `jevx doctor`: 設定と導入状態の診断。

use jevx::{Config, JevxError};

pub(super) fn run_doctor_with_config(json: bool, config: &Config) -> Result<i32, JevxError> {
    let key_configured = config
        .api_key
        .as_ref()
        .is_some_and(|key| !key.trim().is_empty());
    let report = serde_json::json!({
        "schemaVersion": 1,
        "apiKeyConfigured": key_configured,
        "endpoint": config.endpoint,
        "telemetryPath": config.telemetry_path,
        "telemetryEnabled": config.telemetry_enabled,
        "platform": "macos"
    });
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("API key configured: {}", key_configured);
        println!(
            "Endpoint: {}",
            report["endpoint"].as_str().unwrap_or_default()
        );
        println!(
            "Telemetry: {}",
            report["telemetryPath"].as_str().unwrap_or_default()
        );
    }
    Ok(0)
}
