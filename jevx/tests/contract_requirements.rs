//! 公開境界（`--json` のキー、終了コード、エラー文）を実バイナリで固定する契約テスト。
//! キーを消したり名前を変えたりするときは `schemaVersion` を上げ、PHILOSOPHY.md の互換性の約束に従う。

use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use tempfile::tempdir;

const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

fn jevx(home: &Path, cwd: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jevx"));
    command
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .env("HOME", home)
        .env("JEVX_HOME", home.join(".jevx"))
        .env("JEVX_TELEMETRY", "off")
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null());
    // Keep coverage instrumentation writing to cargo-llvm-cov's profile directory.
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    command.output().expect("run jevx")
}

fn json_output(home: &Path, cwd: &Path, args: &[&str]) -> Value {
    let output = jevx(home, cwd, args);
    assert!(
        output.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("json stdout")
}

fn keys(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect()
}

fn assert_has_keys(value: &Value, expected: &[&str]) {
    let actual = keys(value);
    for key in expected {
        assert!(actual.contains(key), "missing public key {key}: {actual:?}");
    }
}

#[test]
fn doctor_json_contract() {
    let home = tempdir().expect("home");
    let report = json_output(home.path(), home.path(), &["doctor", "--json"]);
    assert_eq!(report["schemaVersion"], 2);
    assert_has_keys(
        &report,
        &[
            "schemaVersion",
            "apiKeyConfigured",
            "endpoint",
            "telemetryPath",
            "telemetryEnabled",
            "platform",
            "jevxOnPath",
            "skillInstalled",
            "hooks",
            "thresholds",
            "limits",
            "warnings",
            "nextSteps",
        ],
    );
    assert_has_keys(
        &report["thresholds"],
        &["minProbability", "minMargin", "maxCandidates", "customized"],
    );
    assert_has_keys(
        &report["limits"],
        &[
            "requestTimeoutMs",
            "maxStateBytes",
            "maxRetries",
            "retryBackoffMs",
            "decisionCacheCapacity",
            "inputCostWeight",
            "outputCostWeight",
            "customized",
        ],
    );
    assert_eq!(report["limits"]["customized"], false);
}

#[test]
fn hooks_install_and_uninstall_json_contract_round_trip() {
    let home = tempdir().expect("home");
    let repo = home.path().join("repo");
    std::fs::create_dir_all(&repo).expect("repo");
    let repo_arg = repo.to_str().expect("utf8");
    let install = json_output(
        home.path(),
        &repo,
        &[
            "hooks", "install", "--scope", "project", "--repo", repo_arg, "--json",
        ],
    );
    assert_has_keys(
        &install,
        &[
            "path",
            "changed",
            "dryRun",
            "recordsPath",
            "stateDir",
            "config",
        ],
    );
    let preview = json_output(
        home.path(),
        &repo,
        &[
            "hooks",
            "uninstall",
            "--scope",
            "project",
            "--repo",
            repo_arg,
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(preview["schemaVersion"], 1);
    assert_has_keys(
        &preview,
        &[
            "schemaVersion",
            "path",
            "exists",
            "changed",
            "dryRun",
            "removedHandlers",
            "config",
        ],
    );
    assert_eq!(preview["removedHandlers"], 7);
    let removed = json_output(
        home.path(),
        &repo,
        &[
            "hooks",
            "uninstall",
            "--scope",
            "project",
            "--repo",
            repo_arg,
            "--json",
        ],
    );
    assert_eq!(removed["config"], serde_json::json!({"hooks": {}}));
}

#[test]
fn data_json_contract() {
    let home = tempdir().expect("home");
    let inventory = json_output(home.path(), home.path(), &["data", "path", "--json"]);
    assert_eq!(inventory["schemaVersion"], 6);
    assert_has_keys(&inventory, &["schemaVersion", "dataHome", "files"]);
    assert_has_keys(
        &inventory["files"][0],
        &["kind", "path", "exists", "bytes", "records"],
    );
    let exported = json_output(home.path(), home.path(), &["data", "export"]);
    assert_has_keys(&exported, &["schemaVersion", "dataHome", "records"]);
    let purge = json_output(home.path(), home.path(), &["data", "purge", "--json"]);
    assert_has_keys(&purge, &["schemaVersion", "dryRun", "removed"]);
    assert_eq!(purge["dryRun"], true);
}

#[test]
fn hook_stats_json_contract() {
    let home = tempdir().expect("home");
    let input = home.path().join("hooks.jsonl");
    std::fs::write(
        &input,
        r#"{"schemaVersion":3,"mode":"shadow","hookEventName":"PreCompact","elapsedMs":4}"#,
    )
    .expect("hook records");
    let input_arg = input.to_str().expect("input path");
    let report = json_output(
        home.path(),
        home.path(),
        &["hooks", "stats", "--input", input_arg, "--json"],
    );
    assert_eq!(report["schemaVersion"], 5);
    assert_has_keys(
        &report,
        &[
            "schemaVersion",
            "recordCount",
            "eventCounts",
            "decisionCounts",
            "dedupeHitCount",
            "latencyMsP50",
            "latencyMsP95",
            "tokenSavings",
            "jevCost",
            "codexCost",
            "totalCost",
            "costStatusCounts",
        ],
    );
    assert_has_keys(
        &report["tokenSavings"],
        &[
            "measuredRecords",
            "beforeTokens",
            "afterTokens",
            "savedTokens",
            "reductionRate",
        ],
    );
}

#[test]
fn eval_dry_run_contract_keeps_none_precision_alias_and_adds_none_recall() {
    let home = tempdir().expect("home");
    let repo = Path::new(MANIFEST_DIR).parent().expect("repo root");
    let report = json_output(home.path(), repo, &["eval", "--dry-run", "--json"]);
    assert_eq!(report["schemaVersion"], 2);
    assert_has_keys(
        &report,
        &[
            "schemaVersion",
            "caseCount",
            "baselineMode",
            "modes",
            "comparisons",
            "cases",
        ],
    );
    let summary = report["modes"]
        .as_object()
        .expect("modes")
        .values()
        .next()
        .expect("mode");
    assert_has_keys(
        summary,
        &[
            "status",
            "cases",
            "accuracy",
            "expectedNone",
            "noneCorrect",
            "nonePrecision",
            "noneRecall",
            "errorRate",
        ],
    );
    assert_eq!(summary["noneRecall"], summary["nonePrecision"]);

    // Skills under the working directory or HOME must not leak into the evaluation catalog.
    let elsewhere = tempdir().expect("elsewhere");
    let fixtures = repo.join("jevx/evals/skill-selection.jsonl");
    let catalog = repo.join("jevx/evals/skills");
    let isolated = json_output(
        home.path(),
        elsewhere.path(),
        &[
            "eval",
            "--dry-run",
            "--json",
            "--fixtures",
            fixtures.to_str().expect("utf8"),
            "--skill-dir",
            catalog.to_str().expect("utf8"),
        ],
    );
    // Compare decision quality only; latency fields differ between runs.
    let quality = |report: &Value| {
        report["modes"]
            .as_object()
            .expect("modes")
            .iter()
            .map(|(mode, summary)| {
                let fields = [
                    "cases",
                    "correct",
                    "accuracy",
                    "expectedNone",
                    "noneCorrect",
                    "noneRecall",
                    "candidateMisses",
                    "errors",
                ];
                let picked: Vec<Value> = fields.iter().map(|key| summary[*key].clone()).collect();
                (mode.clone(), picked)
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(quality(&report), quality(&isolated));
}

#[test]
fn eval_outside_repository_explains_how_to_pass_fixture_paths() {
    let home = tempdir().expect("home");
    let output = jevx(home.path(), home.path(), &["eval", "--dry-run", "--json"]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("jevx/evals/skill-selection.jsonl"),
        "{stderr}"
    );
    assert!(stderr.contains("--fixtures"), "{stderr}");
    assert!(stderr.contains("--skill-dir"), "{stderr}");
}

#[test]
fn suggest_error_exit_codes_are_stable() {
    let home = tempdir().expect("home");
    let catalog = Path::new(MANIFEST_DIR).join("evals/skills");
    let catalog = catalog.to_str().expect("utf8");

    // With candidates but no API key, the decision cannot complete: exit 2 and a missing_api_key error.
    let missing_key = jevx(
        home.path(),
        home.path(),
        &[
            "skills",
            "suggest",
            "--prompt",
            "PDFを結合したい",
            "--json",
            "--skill-dir",
            catalog,
        ],
    );
    assert_eq!(missing_key.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&missing_key.stdout).contains("missing_api_key"));

    // With no candidates, Jev is not needed: a normal no_candidates result.
    let no_candidates = json_output(
        home.path(),
        home.path(),
        &["skills", "suggest", "--prompt", "test", "--json"],
    );
    assert_eq!(no_candidates["decision"], "no_candidates");

    let no_input = jevx(home.path(), home.path(), &["skills", "suggest", "--json"]);
    assert_eq!(no_input.status.code(), Some(2));
}
