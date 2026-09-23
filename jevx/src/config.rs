use std::env;
use std::fmt::Display;
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::str::FromStr;
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
    /// 範囲外などで既定値へ戻した環境変数の説明。`doctor` が表示する。
    pub warnings: Vec<String>,
}

/// 意見のある既定値。環境変数での上書きは逃げ道で、既定値の改善を優先する。
pub const DEFAULT_MAX_CANDIDATES: usize = 32;
pub const DEFAULT_MIN_PROBABILITY: f64 = 0.60;
pub const DEFAULT_MIN_MARGIN: f64 = 0.10;

impl Config {
    pub fn from_env() -> Self {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let data_home = env::var_os("JEVX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".jevx"));
        let mut warnings = Vec::new();
        let var = |key: &str| env::var(key).ok();
        Self {
            max_candidates: bounded(
                "JEVX_MAX_CANDIDATES",
                var("JEVX_MAX_CANDIDATES").as_deref(),
                DEFAULT_MAX_CANDIDATES,
                1..=256,
                &mut warnings,
            ),
            min_probability: bounded(
                "JEVX_MIN_PROBABILITY",
                var("JEVX_MIN_PROBABILITY").as_deref(),
                DEFAULT_MIN_PROBABILITY,
                0.0..=1.0,
                &mut warnings,
            ),
            min_margin: bounded(
                "JEVX_MIN_MARGIN",
                var("JEVX_MIN_MARGIN").as_deref(),
                DEFAULT_MIN_MARGIN,
                0.0..=1.0,
                &mut warnings,
            ),
            endpoint: env::var("JEVX_GATEWAY_ENDPOINT")
                .unwrap_or_else(|_| DEFAULT_ENDPOINT.to_owned()),
            api_key: env::var("AI_GATEWAY_API_KEY").ok(),
            timeout: Duration::from_millis(
                env::var("JEVX_REQUEST_TIMEOUT_MS")
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(1_500),
            ),
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
            warnings,
        }
    }

    pub fn for_test(data_home: PathBuf) -> Self {
        Self {
            endpoint: DEFAULT_ENDPOINT.to_owned(),
            api_key: Some("test-key".to_owned()),
            timeout: Duration::from_secs(1),
            max_candidates: DEFAULT_MAX_CANDIDATES,
            min_probability: DEFAULT_MIN_PROBABILITY,
            min_margin: DEFAULT_MIN_MARGIN,
            telemetry_path: data_home.join("events.jsonl"),
            telemetry_enabled: false,
            decision_receipt_path: data_home.join("decisions.jsonl"),
            max_state_bytes: 32_000,
            max_retries: 0,
            retry_backoff_ms: 0,
            cache_capacity: 0,
            input_cost_weight: 1.0,
            output_cost_weight: 1.0,
            warnings: Vec::new(),
        }
    }

    pub fn thresholds_customized(&self) -> bool {
        self.max_candidates != DEFAULT_MAX_CANDIDATES
            || self.min_probability != DEFAULT_MIN_PROBABILITY
            || self.min_margin != DEFAULT_MIN_MARGIN
    }
}

/// `raw` が `range` 内に解釈できればその値、できなければ既定値を使い、理由を `warnings` に残す。
fn bounded<T>(
    name: &str,
    raw: Option<&str>,
    default: T,
    range: RangeInclusive<T>,
    warnings: &mut Vec<String>,
) -> T
where
    T: FromStr + PartialOrd + Display + Copy,
{
    let Some(raw) = raw else {
        return default;
    };
    match raw.trim().parse::<T>() {
        Ok(value) if range.contains(&value) => value,
        _ => {
            warnings.push(format!(
                "{name} must be between {} and {}; using default {default}",
                range.start(),
                range.end()
            ));
            default
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
            "JEVX_MIN_PROBABILITY",
            "JEVX_MIN_MARGIN",
            "JEVX_MAX_CANDIDATES",
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
        set_var("JEVX_MIN_PROBABILITY", "0.7");
        set_var("JEVX_MIN_MARGIN", "0.05");
        set_var("JEVX_MAX_CANDIDATES", "0");
        let configured = Config::from_env();
        assert_eq!(configured.min_probability, 0.7);
        assert_eq!(configured.min_margin, 0.05);
        assert_eq!(configured.max_candidates, DEFAULT_MAX_CANDIDATES);
        assert_eq!(configured.warnings.len(), 1);
        assert!(configured.thresholds_customized());
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
        assert!(defaults.warnings.is_empty());
        assert!(!defaults.thresholds_customized());

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

    #[test]
    fn threshold_overrides_accept_values_in_range_and_warn_otherwise() {
        let mut warnings = Vec::new();
        assert_eq!(
            bounded(
                "JEVX_MIN_PROBABILITY",
                Some("0.75"),
                0.60,
                0.0..=1.0,
                &mut warnings
            ),
            0.75
        );
        assert_eq!(
            bounded(
                "JEVX_MAX_CANDIDATES",
                Some("8"),
                32_usize,
                1..=256,
                &mut warnings
            ),
            8
        );
        assert_eq!(
            bounded("JEVX_MIN_MARGIN", None, 0.10, 0.0..=1.0, &mut warnings),
            0.10
        );
        assert!(warnings.is_empty());

        assert_eq!(
            bounded(
                "JEVX_MIN_PROBABILITY",
                Some("1.5"),
                0.60,
                0.0..=1.0,
                &mut warnings
            ),
            0.60
        );
        assert_eq!(
            bounded(
                "JEVX_MAX_CANDIDATES",
                Some("many"),
                32_usize,
                1..=256,
                &mut warnings
            ),
            32
        );
        assert_eq!(warnings.len(), 2);
        assert!(warnings[0].contains("JEVX_MIN_PROBABILITY"));
        assert!(warnings[0].contains("0.6"));
        assert!(warnings[1].contains("JEVX_MAX_CANDIDATES"));
    }

    #[test]
    fn default_thresholds_are_reported_as_not_customized() {
        let config = Config::for_test(PathBuf::from("/tmp/data"));
        assert!(!config.thresholds_customized());
        let custom = Config {
            min_margin: 0.2,
            ..config
        };
        assert!(custom.thresholds_customized());
    }
}
