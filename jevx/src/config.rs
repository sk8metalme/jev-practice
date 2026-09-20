use std::env;
use std::path::PathBuf;
use std::time::Duration;

use crate::DEFAULT_ENDPOINT;

#[derive(Debug, Clone)]
pub struct Config {
    pub endpoint: String,
    pub api_key: Option<String>,
    pub timeout: Duration,
    pub max_candidates: usize,
    pub min_probability: f64,
    pub min_margin: f64,
    pub telemetry_path: PathBuf,
    pub telemetry_enabled: bool,
}

impl Config {
    pub fn from_env() -> Self {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let data_home = env::var_os("JEVX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".jevx"));
        Self {
            endpoint: env::var("JEVX_GATEWAY_ENDPOINT")
                .unwrap_or_else(|_| DEFAULT_ENDPOINT.to_owned()),
            api_key: env::var("AI_GATEWAY_API_KEY").ok(),
            timeout: Duration::from_millis(
                env::var("JEVX_REQUEST_TIMEOUT_MS")
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(1_500),
            ),
            max_candidates: 32,
            min_probability: 0.60,
            min_margin: 0.10,
            telemetry_path: data_home.join("events.jsonl"),
            telemetry_enabled: env::var("JEVX_TELEMETRY")
                .map(|value| value != "0" && value != "off")
                .unwrap_or(true),
        }
    }

    pub fn for_test(data_home: PathBuf) -> Self {
        Self {
            endpoint: DEFAULT_ENDPOINT.to_owned(),
            api_key: Some("test-key".to_owned()),
            timeout: Duration::from_secs(1),
            max_candidates: 32,
            min_probability: 0.60,
            min_margin: 0.10,
            telemetry_path: data_home.join("events.jsonl"),
            telemetry_enabled: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn set_var(key: &str, value: &str) {
        // Environment mutation is isolated by ENV_LOCK and restored before the test exits.
        unsafe { env::set_var(key, value) };
    }

    fn remove_var(key: &str) {
        unsafe { env::remove_var(key) };
    }

    #[test]
    fn from_env_reads_overrides_and_all_fallbacks() {
        let _guard = ENV_LOCK.lock().expect("environment lock");
        let keys = [
            "HOME",
            "JEVX_HOME",
            "JEVX_GATEWAY_ENDPOINT",
            "AI_GATEWAY_API_KEY",
            "JEVX_REQUEST_TIMEOUT_MS",
            "JEVX_TELEMETRY",
        ];
        let original = keys
            .iter()
            .map(|key| (*key, env::var_os(key)))
            .collect::<Vec<_>>();
        set_var("HOME", "/tmp/jevx-home");
        set_var("JEVX_HOME", "/tmp/jevx-data");
        set_var("JEVX_GATEWAY_ENDPOINT", "http://gateway.test");
        set_var("AI_GATEWAY_API_KEY", "test-key");
        set_var("JEVX_REQUEST_TIMEOUT_MS", "42");
        set_var("JEVX_TELEMETRY", "off");
        let configured = Config::from_env();
        assert_eq!(configured.endpoint, "http://gateway.test");
        assert_eq!(configured.api_key.as_deref(), Some("test-key"));
        assert_eq!(configured.timeout.as_millis(), 42);
        assert_eq!(
            configured.telemetry_path,
            PathBuf::from("/tmp/jevx-data/events.jsonl")
        );
        assert!(!configured.telemetry_enabled);

        for key in keys {
            remove_var(key);
        }
        let defaults = Config::from_env();
        assert_eq!(defaults.endpoint, DEFAULT_ENDPOINT);
        assert!(defaults.api_key.is_none());
        assert_eq!(defaults.timeout.as_millis(), 1_500);
        assert_eq!(
            defaults.telemetry_path,
            PathBuf::from("./.jevx/events.jsonl")
        );
        assert!(defaults.telemetry_enabled);

        set_var("JEVX_REQUEST_TIMEOUT_MS", "not-a-number");
        set_var("JEVX_TELEMETRY", "custom-value");
        let invalid_timeout = Config::from_env();
        assert_eq!(invalid_timeout.timeout.as_millis(), 1_500);
        assert!(invalid_timeout.telemetry_enabled);

        for (key, value) in original {
            if let Some(value) = value {
                unsafe { env::set_var(key, value) };
            } else {
                remove_var(key);
            }
        }
    }
}
