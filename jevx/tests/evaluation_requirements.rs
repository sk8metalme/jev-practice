use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

use async_trait::async_trait;
use jevx::evaluation::{EvaluationFixture, evaluate, evaluate_repeated, load_fixtures};
use jevx::{Config, JevxError, Judge, JudgeRequest, JudgeResponse, SkillRecord};
use tempfile::tempdir;

struct StubJudge {
    response: JudgeResponse,
    requests: Mutex<Vec<JudgeRequest>>,
}

#[async_trait]
impl Judge for StubJudge {
    async fn evaluate(&self, request: JudgeRequest) -> Result<JudgeResponse, JevxError> {
        self.requests.lock().expect("requests lock").push(request);
        Ok(self.response.clone())
    }
}

struct ErrorJudge;

#[async_trait]
impl Judge for ErrorJudge {
    async fn evaluate(&self, _request: JudgeRequest) -> Result<JudgeResponse, JevxError> {
        Err(JevxError::Provider("stub provider failure".to_owned()))
    }
}

fn skill(root: &std::path::Path, id: &str, description: &str) -> SkillRecord {
    SkillRecord::new(
        id.to_owned(),
        description.to_owned(),
        root.join(id).join("SKILL.md"),
        "eval".to_owned(),
    )
}

#[test]
fn fixture_loader_parses_jsonl_and_rejects_empty_cases() {
    let root = tempdir().expect("tempdir");
    let path = root.path().join("fixtures.jsonl");
    std::fs::write(
        &path,
        "\n{\"id\":\"case-1\",\"kind\":\"synthetic\",\"prompt\":\"PDF\",\"expected\":\"pdf\",\"keywords\":[\"PDF\"]}\n",
    )
    .expect("write fixture");
    let fixtures = load_fixtures(&path).expect("fixtures");
    assert_eq!(fixtures.len(), 1);
    assert_eq!(fixtures[0].id, "case-1");

    let empty_path = root.path().join("empty.jsonl");
    std::fs::write(&empty_path, "\n").expect("write empty fixture");
    assert!(matches!(
        load_fixtures(&empty_path),
        Err(JevxError::InvalidInput(message)) if message.contains("at least one")
    ));

    let duplicate_path = root.path().join("duplicate.jsonl");
    let fixture = r#"{"id":"same","kind":"synthetic","prompt":"PDF","expected":"pdf"}"#;
    std::fs::write(&duplicate_path, format!("{fixture}\n{fixture}\n")).expect("write duplicate");
    assert!(matches!(
        load_fixtures(&duplicate_path),
        Err(JevxError::InvalidInput(message)) if message.contains("duplicate fixture id")
    ));
}

#[tokio::test]
async fn evaluation_reports_baselines_and_does_not_serialize_prompt() {
    let root = tempdir().expect("tempdir");
    let fixtures = vec![
        EvaluationFixture {
            id: "pdf-case".to_owned(),
            kind: "synthetic".to_owned(),
            prompt: "PDFを結合したい secret-prompt".to_owned(),
            expected: "pdf".to_owned(),
            keywords: vec!["PDF".to_owned()],
        },
        EvaluationFixture {
            id: "none-case".to_owned(),
            kind: "synthetic".to_owned(),
            prompt: "単純なtypoだけ secret-none".to_owned(),
            expected: "none".to_owned(),
            keywords: vec!["typo".to_owned()],
        },
    ];
    let skills = vec![
        skill(root.path(), "pdf", "PDF pdf 結合 merge"),
        skill(root.path(), "testing", "テスト test testing"),
    ];
    let report = evaluate(
        &fixtures,
        &skills,
        &Config::for_test(root.path().join("data")),
        None,
    )
    .await;

    assert_eq!(report.case_count, 2);
    assert_eq!(report.modes["none"].accuracy, Some(0.5));
    assert_eq!(report.modes["local_keyword"].accuracy, Some(1.0));
    assert_eq!(report.modes["jevx"].status, "not_run");
    let json = serde_json::to_string(&report).expect("report json");
    assert!(!json.contains("secret-prompt"));
    assert!(!json.contains("secret-none"));
    assert!(!json.contains("keywords"));
}

#[tokio::test]
async fn evaluation_reports_local_rank_metrics_and_repeat_distribution() {
    let root = tempdir().expect("tempdir");
    let fixtures = vec![EvaluationFixture {
        id: "pdf-case".to_owned(),
        kind: "synthetic".to_owned(),
        prompt: "PDFを結合したい".to_owned(),
        expected: "pdf".to_owned(),
        keywords: vec!["PDF".to_owned()],
    }];
    let skills = vec![skill(root.path(), "pdf", "PDF pdf 結合 merge")];
    let config = Config::for_test(root.path().join("data"));

    let report = evaluate(&fixtures, &skills, &config, None).await;
    let local_rank = &report.modes["local_rank"];
    assert_eq!(local_rank.accuracy, Some(1.0));
    assert!(local_rank.discovery_ms_p95.is_some());
    assert_eq!(
        report.cases[0].local_rank.prediction.as_deref(),
        Some("pdf")
    );
    assert!(report.cases[0].local_rank.discovery_ms <= 100);

    let repeated = evaluate_repeated(&fixtures, &skills, &config, None, 2)
        .await
        .expect("repeat evaluation");
    assert_eq!(repeated.run_count, 2);
    assert_eq!(repeated.modes["local_rank"].runs, 2);
    assert_eq!(repeated.modes["local_rank"].accuracy.mean, Some(1.0));
    let json = serde_json::to_string(&repeated).expect("repeat json");
    assert!(!json.contains("PDFを結合したい"));
    assert!(!json.contains("keywords"));

    let judge = StubJudge {
        response: JudgeResponse::selected("pdf", 0.95, 17, Some((31, 7))),
        requests: Mutex::new(Vec::new()),
    };
    let repeated_jev = evaluate_repeated(&fixtures, &skills, &config, Some(&judge), 2)
        .await
        .expect("live repeat evaluation");
    let jevx = &repeated_jev.modes["jevx"];
    assert_eq!(jevx.accuracy.mean, Some(1.0));
    assert_eq!(jevx.jev_response_ms.mean, Some(17.0));
    assert_eq!(jevx.input_tokens.mean, Some(31.0));
    assert!(jevx.discovery_ms.p95.is_some());
    assert!(
        evaluate_repeated(&fixtures, &skills, &config, None, 0)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn evaluation_records_jev_metrics_and_writes_safe_case_fields() {
    let root = tempdir().expect("tempdir");
    let fixtures = vec![EvaluationFixture {
        id: "pdf-case".to_owned(),
        kind: "synthetic".to_owned(),
        prompt: "PDFを結合したい".to_owned(),
        expected: "pdf".to_owned(),
        keywords: vec!["PDF".to_owned()],
    }];
    let skills = vec![skill(root.path(), "pdf", "PDF pdf 結合 merge")];
    let judge = StubJudge {
        response: JudgeResponse::selected("pdf", 0.95, 17, Some((31, 7))),
        requests: Mutex::new(Vec::new()),
    };
    let report = evaluate(
        &fixtures,
        &skills,
        &Config::for_test(root.path().join("data")),
        Some(&judge),
    )
    .await;

    let mode = &report.modes["jevx"];
    assert_eq!(mode.status, "completed");
    assert_eq!(mode.accuracy, Some(1.0));
    assert_eq!(mode.jev_response_ms_p50, Some(17));
    assert_eq!(mode.jev_response_ms_p95, Some(17));
    assert_eq!(mode.average_input_tokens, Some(31.0));
    assert_eq!(mode.average_output_tokens, Some(7.0));
    assert_eq!(
        report.cases[0].jevx.as_ref().unwrap().selected.as_deref(),
        Some("pdf")
    );
    assert_eq!(judge.requests.lock().expect("requests lock").len(), 1);
    let request = judge.requests.lock().expect("requests lock");
    assert!(!request[0].state.contains("expected"));
    assert!(!request[0].state.contains("keywords"));

    let output = root.path().join("results.jsonl");
    jevx::evaluation::write_case_results(&output, &report.cases).expect("write results");
    let written = std::fs::read_to_string(output).expect("read results");
    assert!(written.contains("pdf-case"));
    assert!(!written.contains("PDFを結合したい"));
}

#[tokio::test]
async fn evaluation_records_provider_errors_without_stopping_the_run() {
    let root = tempdir().expect("tempdir");
    let fixtures = vec![EvaluationFixture {
        id: "error-case".to_owned(),
        kind: "synthetic".to_owned(),
        prompt: "PDFを確認したい".to_owned(),
        expected: "pdf".to_owned(),
        keywords: vec!["PDF".to_owned()],
    }];
    let skills = vec![skill(root.path(), "pdf", "PDF pdf 確認")];
    let report = evaluate(
        &fixtures,
        &skills,
        &Config::for_test(root.path().join("data")),
        Some(&ErrorJudge),
    )
    .await;

    let mode = &report.modes["jevx"];
    assert_eq!(mode.errors, 1);
    assert_eq!(mode.error_rate, Some(1.0));
    assert_eq!(
        report.cases[0].jevx.as_ref().unwrap().error_code.as_deref(),
        Some("provider_error")
    );
}

#[test]
fn invalid_fixture_line_includes_line_number() {
    let root = tempdir().expect("tempdir");
    let path = root.path().join("invalid.jsonl");
    std::fs::write(&path, "\nnot-json\n").expect("write fixture");
    assert!(matches!(
        load_fixtures(&path),
        Err(JevxError::InvalidInput(message)) if message.contains("line 2")
    ));
}

#[test]
fn eval_dry_run_cli_has_all_modes_and_safe_output() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new(env!("CARGO_BIN_EXE_jevx"))
        .args([
            "eval",
            "--fixtures",
            "jevx/evals/skill-selection.jsonl",
            "--skill-dir",
            "jevx/evals/skills",
            "--dry-run",
            "--json",
        ])
        .current_dir(manifest.parent().expect("repo root"))
        .output()
        .expect("run eval");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("report json");
    assert_eq!(report["caseCount"], 40);
    assert_eq!(report["modes"]["jevx"]["status"], "not_run");
    assert!(report["modes"]["local_keyword"]["accuracy"].is_number());
    assert!(report["cases"][0].get("prompt").is_none());
}
