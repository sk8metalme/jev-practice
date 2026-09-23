//! `jevx stats`: ローカルTelemetryの集計。

use std::path::PathBuf;

use jevx::{Config, JevxError, read_stats};

use super::*;

pub(super) fn run_stats_with_config(
    json: bool,
    input: Option<PathBuf>,
    config: &Config,
) -> Result<i32, JevxError> {
    let path = input.unwrap_or_else(|| config.telemetry_path.clone());
    let stats = read_stats(&path)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        print!("{}", stats_human_output(&stats));
    }
    Ok(0)
}

pub(super) fn stats_human_output(stats: &jevx::Stats) -> String {
    format!(
        "events: {}\nselected: {}\nnone: {}\nerrors: {}\nselected rate: {}\nnone rate: {}\nerror rate: {}\naverage Jev response: {:.1} ms\nJev response p50/p95: {}/{} ms\nTotal p50/p95: {}/{} ms\naverage input/output tokens: {}/{}\nusage events: {}\n",
        stats.events,
        stats.selected,
        stats.none,
        stats.errors,
        format_ratio(stats.selected_rate),
        format_ratio(stats.none_rate),
        format_ratio(stats.error_rate),
        stats.average_jev_response_ms,
        format_optional_u64(stats.jev_response_ms_p50),
        format_optional_u64(stats.jev_response_ms_p95),
        format_optional_u64(stats.total_ms_p50),
        format_optional_u64(stats.total_ms_p95),
        format_optional_f64(stats.average_input_tokens),
        format_optional_f64(stats.average_output_tokens),
        stats.usage_events,
    )
}
