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
    pub decision_receipt_path: PathBuf,
    pub max_state_bytes: usize,
    pub max_retries: usize,
    pub retry_backoff_ms: u64,
    pub cache_capacity: usize,
    pub input_cost_weight: f64,
    pub output_cost_weight: f64,
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
            decision_receipt_path: data_home.join("decisions.jsonl"),
            max_state_bytes: env::var("JEVX_MAX_STATE_BYTES")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(32_000),
            max_retries: env::var("JEVX_MAX_RETRIES")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1),
            retry_backoff_ms: env::var("JEVX_RETRY_BACKOFF_MS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(25),
            cache_capacity: env::var("JEVX_DECISION_CACHE_CAPACITY")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
            input_cost_weight: cost_weight("JEVX_INPUT_COST_WEIGHT"),
            output_cost_weight: cost_weight("JEVX_OUTPUT_COST_WEIGHT"),
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
            decision_receipt_path: data_home.join("decisions.jsonl"),
            max_state_bytes: 32_000,
            max_retries: 0,
            retry_backoff_ms: 0,
            cache_capacity: 0,
            input_cost_weight: 1.0,
            output_cost_weight: 1.0,
        }
    }
}

fn cost_weight(key: &str) -> f64 {
    env::var(key)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(1.0)
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
            "JEVX_MAX_STATE_BYTES",
            "JEVX_MAX_RETRIES",
            "JEVX_RETRY_BACKOFF_MS",
            "JEVX_DECISION_CACHE_CAPACITY",
            "JEVX_INPUT_COST_WEIGHT",
            "JEVX_OUTPUT_COST_WEIGHT",
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
        set_var("JEVX_MAX_STATE_BYTES", "2048");
        set_var("JEVX_MAX_RETRIES", "3");
        set_var("JEVX_RETRY_BACKOFF_MS", "7");
        set_var("JEVX_DECISION_CACHE_CAPACITY", "5");
        set_var("JEVX_INPUT_COST_WEIGHT", "1.5");
        set_var("JEVX_OUTPUT_COST_WEIGHT", "2.5");
        let configured = Config::from_env();
        assert_eq!(configured.endpoint, "http://gateway.test");
        assert_eq!(configured.api_key.as_deref(), Some("test-key"));
        assert_eq!(configured.timeout.as_millis(), 42);
        assert_eq!(
            configured.telemetry_path,
            PathBuf::from("/tmp/jevx-data/events.jsonl")
        );
        assert!(!configured.telemetry_enabled);
        assert_eq!(configured.max_state_bytes, 2048);
        assert_eq!(configured.max_retries, 3);
        assert_eq!(configured.retry_backoff_ms, 7);
        assert_eq!(configured.cache_capacity, 5);
        assert_eq!(configured.input_cost_weight, 1.5);
        assert_eq!(configured.output_cost_weight, 2.5);

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
        assert_eq!(defaults.max_state_bytes, 32_000);
        assert_eq!(defaults.max_retries, 1);
        assert_eq!(defaults.retry_backoff_ms, 25);
        assert_eq!(defaults.cache_capacity, 0);
        assert_eq!(defaults.input_cost_weight, 1.0);
        assert_eq!(defaults.output_cost_weight, 1.0);

        set_var("JEVX_REQUEST_TIMEOUT_MS", "not-a-number");
        set_var("JEVX_TELEMETRY", "custom-value");
        set_var("JEVX_INPUT_COST_WEIGHT", "-1");
        set_var("JEVX_OUTPUT_COST_WEIGHT", "not-a-number");
        let invalid_timeout = Config::from_env();
        assert_eq!(invalid_timeout.timeout.as_millis(), 1_500);
        assert!(invalid_timeout.telemetry_enabled);
        assert_eq!(invalid_timeout.input_cost_weight, 1.0);
        assert_eq!(invalid_timeout.output_cost_weight, 1.0);

        for (key, value) in original {
            if let Some(value) = value {
                unsafe { env::set_var(key, value) };
            } else {
                remove_var(key);
            }
        }
    }
}
