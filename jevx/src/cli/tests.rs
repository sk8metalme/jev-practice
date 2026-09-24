use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::net::TcpListener;
use std::time::Duration;

use super::*;
use jevx::evaluation::{
    DistributionSummary, EvaluationReport, ModeSummary, RepeatEvaluationReport, RepeatModeSummary,
};
use jevx::hook_config::HookScope;
use jevx::{CandidateDecision, SuggestionResult};
use tempfile::tempdir;

fn suggest_args() -> SuggestArgs {
    SuggestArgs {
        prompt: Some("use pdf".to_owned()),
        stdin: false,
        input_json: false,
        json: false,
        cwd: None,
        explicit_skill: Some("pdf".to_owned()),
        skill_dirs: Vec::new(),
        no_telemetry: false,
    }
}

#[test]
fn stats_human_output_includes_token_averages_and_home_requirements() {
    let stats = jevx::Stats {
        average_input_tokens: Some(110.0),
        average_output_tokens: Some(22.5),
        usage_events: 2,
        ..jevx::Stats::default()
    };
    let output = stats_human_output(&stats);
    assert!(output.contains("average input/output tokens: 110.0/22.5"));
    assert!(resolve_hook_paths(HookScope::User, None, None).is_err());
    assert_eq!(
        resolve_hook_paths(HookScope::Project, None, None)
            .expect("project paths")
            .0,
        PathBuf::from(".")
    );
}

#[test]
fn review_content_never_includes_skill_or_settings() {
    let payload = serde_json::json!({
        "prompt": "prompt fixture",
        "plan": "plan fixture",
        "diff": "diff fixture",
        "finalAnswer": "answer fixture",
        "skillBody": "skill secret=fixture-only",
        "settings": {"mode": "fixture"},
        "toolResult": "must never be selected",
    });
    let without_opt_in =
        review_content(&payload, jevx::ReviewTarget::Turn, false).expect("local review content");
    assert!(without_opt_in.contains("prompt fixture"));
    assert!(!without_opt_in.contains("skill secret"));
    assert!(!without_opt_in.contains("must never be selected"));
    let with_opt_in =
        review_content(&payload, jevx::ReviewTarget::Turn, true).expect("opt-in review content");
    assert!(with_opt_in.contains("prompt fixture"));
    assert!(!with_opt_in.contains("skill secret"));
    assert!(!with_opt_in.contains("skillBody"));
    assert!(!with_opt_in.contains("settings"));
    assert!(!with_opt_in.contains("must never be selected"));
}

#[tokio::test]
async fn review_cli_records_safe_receipt_and_requires_fix_confirmation() {
    let root = tempdir().expect("tempdir");
    let output = root.path().join("reviews.jsonl");
    let config = Config::for_test(root.path().join("data"));
    let args = ReviewArgs {
        target: ReviewTargetArg::Turn,
        allow_content: false,
        auto_fix: false,
        yes: false,
        cwd: Some(root.path().to_path_buf()),
        output: Some(output.clone()),
        _jevx_managed: false,
    };
    let mut input = Cursor::new(
        r#"{"hook_event_name":"UserPromptSubmit","content":"適宜対応 secret=fixture-only","toolResult":"do not send","taskId":"task","sessionId":"session","turnId":"turn","skillBody":"skill body","settings":{"unsafe":"not sent"}}"#
            .as_bytes()
            .to_vec(),
    );
    assert_eq!(
        run_hook_review_from_reader(args, &config, &mut input)
            .await
            .expect("review"),
        0
    );
    let receipt = fs::read_to_string(&output).expect("receipt");
    assert!(receipt.contains("content_opt_in_required"));
    assert!(!receipt.contains("fixture-only"));
    assert!(!receipt.contains("unsafe"));

    let invalid_fix_args = ReviewArgs {
        target: ReviewTargetArg::Diff,
        allow_content: false,
        auto_fix: true,
        yes: false,
        cwd: Some(root.path().to_path_buf()),
        output: Some(root.path().join("invalid-fix.jsonl")),
        _jevx_managed: false,
    };
    let mut invalid_input = Cursor::new(br#"{"diff":"return false;"}"#.to_vec());
    assert!(matches!(
        run_hook_review_from_reader(invalid_fix_args, &config, &mut invalid_input).await,
        Err(JevxError::InvalidInput(message)) if message.contains("--yes")
    ));

    let stats_args = ReviewStatsArgs {
        input: output,
        json: true,
    };
    assert_eq!(run_review_stats(stats_args).expect("stats"), 0);
}

fn write_skill(root: &Path, name: &str) {
    let directory = root.join(name);
    fs::create_dir_all(&directory).expect("mkdir");
    fs::write(
        directory.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {name} helper\n---\n"),
    )
    .expect("skill");
}

#[tokio::test]
async fn run_inner_covers_lists_stats_doctor_and_success_output_modes() {
    let root = tempdir().expect("tempdir");
    write_skill(&root.path().join(".agents/skills"), "pdf");
    let config = Config::for_test(root.path().join("data"));
    let list_json = Cli {
        command: Command::Skills {
            command: SkillsCommand::List(ListArgs {
                json: true,
                cwd: Some(root.path().to_path_buf()),
                skill_dirs: vec![],
            }),
        },
    };
    assert_eq!(run_inner(list_json).await.expect("list"), 0);
    let list_human = Cli {
        command: Command::Skills {
            command: SkillsCommand::List(ListArgs {
                json: false,
                cwd: Some(root.path().to_path_buf()),
                skill_dirs: vec![root.path().to_path_buf()],
            }),
        },
    };
    assert_eq!(run_inner(list_human).await.expect("list"), 0);
    assert_eq!(
        run_with_cli(Cli {
            command: Command::Stats {
                json: true,
                input: None,
            },
        })
        .await,
        0
    );
    assert_eq!(
        run_with_cli(Cli {
            command: Command::Skills {
                command: SkillsCommand::Suggest(SuggestArgs {
                    prompt: None,
                    stdin: false,
                    input_json: false,
                    json: false,
                    cwd: None,
                    explicit_skill: None,
                    skill_dirs: vec![],
                    no_telemetry: false,
                }),
            },
        })
        .await,
        2
    );

    let stats_json = run_stats_with_config(true, None, &config).expect("stats");
    assert_eq!(stats_json, 0);
    let stats_human = run_stats_with_config(false, None, &config).expect("stats");
    assert_eq!(stats_human, 0);
    let doctor_json = run_doctor_with_config(true, &config).expect("doctor");
    assert_eq!(doctor_json, 0);
    let doctor_human = run_doctor_with_config(false, &config).expect("doctor");
    assert_eq!(doctor_human, 0);
    assert_eq!(
        run_inner_with_config(
            Cli {
                command: Command::Stats {
                    json: true,
                    input: None,
                },
            },
            config.clone(),
        )
        .await
        .expect("stats"),
        0
    );
    assert_eq!(
        run_inner_with_config(
            Cli {
                command: Command::Doctor { json: true },
            },
            config.clone(),
        )
        .await
        .expect("doctor"),
        0
    );

    let mut args = suggest_args();
    args.cwd = Some(root.path().to_path_buf());
    let suggest = Cli {
        command: Command::Skills {
            command: SkillsCommand::Suggest(args),
        },
    };
    assert_eq!(
        run_inner_with_config(suggest, config.clone())
            .await
            .expect("suggest"),
        0
    );
    let mut telemetry_config = config.clone();
    telemetry_config.telemetry_enabled = true;
    telemetry_config.telemetry_path = root.path().join("telemetry/events.jsonl");
    let mut telemetry_args = suggest_args();
    telemetry_args.cwd = Some(root.path().to_path_buf());
    assert_eq!(
        run_suggest_with_config(telemetry_args, telemetry_config.clone())
            .await
            .expect("telemetry success"),
        0
    );
    assert!(telemetry_config.telemetry_path.exists());
    let mut json_args = suggest_args();
    json_args.cwd = Some(root.path().to_path_buf());
    json_args.json = true;
    assert_eq!(
        run_suggest_with_config(json_args, config.clone())
            .await
            .expect("suggest"),
        0
    );
    let mut error_config = telemetry_config;
    error_config.api_key = None;
    let mut error_args = suggest_args();
    error_args.cwd = Some(root.path().to_path_buf());
    error_args.json = true;
    error_args.explicit_skill = None;
    assert_eq!(
        run_suggest_with_config(error_args, error_config.clone())
            .await
            .expect("json error"),
        2
    );
    let mut human_error_args = suggest_args();
    human_error_args.cwd = Some(root.path().to_path_buf());
    human_error_args.explicit_skill = None;
    let error = run_suggest_with_config(human_error_args, error_config)
        .await
        .expect_err("human error");
    assert!(matches!(error, JevxError::MissingApiKey));
    let mut skipped_telemetry_args = suggest_args();
    skipped_telemetry_args.cwd = Some(root.path().to_path_buf());
    skipped_telemetry_args.json = true;
    skipped_telemetry_args.no_telemetry = true;
    skipped_telemetry_args.explicit_skill = None;
    let mut skipped_telemetry_config = config;
    skipped_telemetry_config.api_key = None;
    assert_eq!(
        run_suggest_with_config(skipped_telemetry_args, skipped_telemetry_config)
            .await
            .expect("json error without telemetry"),
        2
    );
}

#[tokio::test]
async fn eval_covers_live_error_output_file_and_human_metrics() {
    let root = tempdir().expect("tempdir");
    let skill_dir = root.path().join("skills");
    write_skill(&skill_dir, "pdf");
    let fixture_path = root.path().join("fixtures.jsonl");
    fs::write(
        &fixture_path,
        r#"{"id":"case-1","kind":"synthetic","prompt":"PDFを確認したい","expected":"pdf","keywords":["PDF"]}"#,
    )
    .expect("fixture");
    let output_path = root.path().join("results.jsonl");
    let mut config = Config::for_test(root.path().join("data"));
    config.endpoint = "http://127.0.0.1:1".to_owned();
    config.timeout = Duration::from_millis(10);
    let live_args = EvalArgs {
        fixtures: fixture_path.clone(),
        skill_dirs: vec![skill_dir.clone()],
        cwd: Some(root.path().to_path_buf()),
        dry_run: false,
        json: false,
        output: Some(output_path.clone()),
    };
    assert_eq!(
        run_eval_with_config(live_args, config.clone())
            .await
            .expect("live evaluation"),
        0
    );
    assert!(output_path.exists());
    assert!(
        !fs::read_to_string(&output_path)
            .expect("results")
            .contains("PDFを確認したい")
    );

    let dry_human_args = EvalArgs {
        fixtures: fixture_path,
        skill_dirs: vec![skill_dir],
        cwd: Some(root.path().to_path_buf()),
        dry_run: true,
        json: false,
        output: None,
    };
    assert_eq!(
        run_eval_with_config(dry_human_args, config)
            .await
            .expect("dry evaluation"),
        0
    );

    let mut modes = BTreeMap::new();
    modes.insert(
        "metrics".to_owned(),
        ModeSummary {
            status: "completed".to_owned(),
            cases: 1,
            correct: 1,
            accuracy: Some(1.0),
            expected_none: 0,
            none_correct: 0,
            none_precision: None,
            none_recall: None,
            candidate_misses: 0,
            candidate_miss_rate: Some(0.0),
            errors: 0,
            error_rate: Some(0.0),
            fallback_rate: Some(0.0),
            cache_hit_rate: Some(0.0),
            retry_rate: Some(0.0),
            average_retries: Some(0.0),
            average_relative_cost: Some(0.0),
            jev_response_ms_p50: Some(12),
            jev_response_ms_p95: Some(18),
            total_ms_p50: Some(15),
            total_ms_p95: Some(22),
            discovery_ms_p50: Some(9),
            discovery_ms_p95: Some(11),
            average_input_tokens: Some(8.0),
            average_output_tokens: Some(2.0),
            jev_cost: Some(0.1),
            codex_cost: None,
            total_cost: None,
            cost_status_counts: BTreeMap::new(),
        },
    );
    print_evaluation_human(&EvaluationReport {
        schema_version: 2,
        case_count: 1,
        baseline_mode: "local_rank".to_owned(),
        modes,
        comparisons: BTreeMap::new(),
        cases: Vec::new(),
    });
    assert_eq!(format_ratio(Some(0.5)), "0.500");
    assert_eq!(format_ratio(None), "n/a");
}

#[tokio::test]
async fn repeat_and_hook_commands_write_safe_reports() {
    let root = tempdir().expect("tempdir");
    let skill_dir = root.path().join("skills");
    write_skill(&skill_dir, "pdf");
    let fixture_path = root.path().join("fixtures.jsonl");
    fs::write(
        &fixture_path,
        r#"{"id":"case-1","kind":"synthetic","prompt":"PDF","expected":"pdf"}"#,
    )
    .expect("fixture");
    let repeat_output = root.path().join("repeat.json");
    let repeat_args = EvalRepeatArgs {
        runs: 2,
        fixtures: fixture_path.clone(),
        skill_dirs: vec![skill_dir.clone()],
        cwd: Some(root.path().to_path_buf()),
        dry_run: true,
        json: true,
        output: Some(repeat_output.clone()),
    };
    assert_eq!(
        run_eval_repeat_with_config(repeat_args, Config::for_test(root.path().join("data")))
            .await
            .expect("repeat"),
        0
    );
    let repeat_json = fs::read_to_string(repeat_output).expect("repeat report");
    assert!(repeat_json.contains("local_rank"));
    assert!(!repeat_json.contains("PDF"));

    let human_repeat_args = EvalRepeatArgs {
        runs: 1,
        fixtures: fixture_path,
        skill_dirs: vec![skill_dir],
        cwd: Some(root.path().to_path_buf()),
        dry_run: true,
        json: false,
        output: None,
    };
    assert_eq!(
        run_eval_repeat_with_config(
            human_repeat_args,
            Config::for_test(root.path().join("data")),
        )
        .await
        .expect("human repeat"),
        0
    );

    let mut live_repeat_config = Config::for_test(root.path().join("data"));
    live_repeat_config.endpoint = "http://127.0.0.1:1".to_owned();
    assert_eq!(
        run_eval_repeat_with_config(
            EvalRepeatArgs {
                runs: 1,
                fixtures: root.path().join("fixtures.jsonl"),
                skill_dirs: vec![root.path().join("skills")],
                cwd: Some(root.path().to_path_buf()),
                dry_run: false,
                json: true,
                output: None,
            },
            live_repeat_config,
        )
        .await
        .expect("live repeat with provider errors"),
        0
    );

    let hook_output = root.path().join("hooks.jsonl");
    let hook_args = HookShadowArgs {
        event: Some("PreCompact".to_owned()),
        cwd: Some(root.path().to_path_buf()),
        skill_dirs: vec![],
        output: Some(hook_output.clone()),
        _jevx_managed: false,
    };
    let mut hook_input =
        Cursor::new(br#"{"hook_event_name":"PreCompact","trigger":"manual"}"#.to_vec());
    assert_eq!(
        run_hook_shadow_from_reader(
            hook_args,
            Config::for_test(root.path().join("data")),
            &mut hook_input,
        )
        .await
        .expect("hook"),
        0
    );
    assert!(
        fs::read_to_string(hook_output)
            .expect("hook output")
            .contains("PreCompact")
    );

    let compact_output = root.path().join("compact.json");
    assert_eq!(
        run_compact_eval(CompactEvalArgs {
            runs: 2,
            json: true,
            output: Some(compact_output.clone()),
        })
        .expect("compact json"),
        0
    );
    assert!(
        fs::read_to_string(compact_output)
            .expect("compact report")
            .contains("secretLeaks")
    );
    assert_eq!(
        run_compact_eval(CompactEvalArgs {
            runs: 1,
            json: false,
            output: None,
        })
        .expect("compact human"),
        0
    );

    let correlation_input = root.path().join("hook-correlation.jsonl");
    fs::write(
        &correlation_input,
        r#"{"schemaVersion":1,"mode":"shadow","hookEventName":"UserPromptSubmit","sessionIdSha256":"session-hash","turnIdSha256":"turn-hash","modelSha256":"model-hash","elapsedMs":1}
{"schemaVersion":1,"mode":"shadow","hookEventName":"UserPromptSubmit","sessionIdSha256":"session-hash","turnIdSha256":"turn-hash","modelSha256":"model-hash","elapsedMs":2}"#,
    )
    .expect("correlation fixture");
    let correlation_output = root.path().join("hook-correlation.json");
    assert_eq!(
        run_hook_correlation(CorrelationArgs {
            input: correlation_input.clone(),
            json: true,
            output: Some(correlation_output.clone()),
        })
        .expect("correlation json"),
        0
    );
    let correlation_json = fs::read_to_string(&correlation_output).expect("correlation report");
    assert!(correlation_json.contains("duplicateGroupCount"));
    assert_eq!(
        run_hook_correlation(CorrelationArgs {
            input: correlation_input.clone(),
            json: false,
            output: None,
        })
        .expect("correlation human"),
        0
    );

    let conversation_input = root.path().join("conversation.jsonl");
    fs::write(
        &conversation_input,
        r#"{"caseId":"cli-case","model":"gpt-5.6-sol","requiredFacts":["goal=keep","next=verify"],"followUpText":"goal=keep\nnext=verify\ndecoy_marker=redacted","secretMarkers":["CLI_SECRET_FIXTURE"],"failureRecoveryRequired":true,"toolHistoryItems":2,"toolFailureCount":1,"interruptedTurns":1,"recoveryTurns":1,"recoveryCompleted":true,"preCompactionUsage":{"inputTokens":10,"cachedInputTokens":4,"outputTokens":1,"totalTokens":11},"compactionUsage":{"inputTokens":6,"cachedInputTokens":2,"outputTokens":1,"totalTokens":7},"postCompactionUsage":{"inputTokens":4,"cachedInputTokens":2,"outputTokens":1,"totalTokens":5},"compactionCompleted":true,"compactionDurationMs":12,"inputChars":40,"conversationTurns":4,"observedEvents":["contextCompaction","functionCallOutput","turn/interrupted","turn/completed"]}"#,
    )
    .expect("conversation fixture");
    let conversation_output = root.path().join("conversation.json");
    assert_eq!(
        run_conversation_eval(ConversationEvalArgs {
            input: conversation_input.clone(),
            json: true,
            output: Some(conversation_output.clone()),
        })
        .expect("conversation json"),
        0
    );
    let conversation_json = fs::read_to_string(&conversation_output).expect("conversation report");
    assert!(conversation_json.contains("compactionCompletionRate"));
    assert!(!conversation_json.contains("CLI_SECRET_FIXTURE"));
    assert_eq!(
        run_conversation_eval(ConversationEvalArgs {
            input: conversation_input.clone(),
            json: false,
            output: None,
        })
        .expect("conversation human"),
        0
    );

    let command_repeat = Cli {
        command: Command::EvalRepeat(EvalRepeatArgs {
            runs: 1,
            fixtures: root.path().join("missing-fixture.jsonl"),
            skill_dirs: vec![],
            cwd: None,
            dry_run: true,
            json: true,
            output: None,
        }),
    };
    assert!(
        run_inner_with_config(command_repeat, Config::for_test(root.path().join("data")))
            .await
            .is_err()
    );

    let command_hooks = Cli {
        command: Command::Hooks {
            command: HooksCommand::CompactEval(CompactEvalArgs {
                runs: 1,
                json: true,
                output: None,
            }),
        },
    };
    assert_eq!(
        run_inner_with_config(command_hooks, Config::for_test(root.path().join("data")))
            .await
            .expect("hooks command"),
        0
    );

    let command_conversation = Cli {
        command: Command::Hooks {
            command: HooksCommand::ConversationEval(ConversationEvalArgs {
                input: conversation_input,
                json: true,
                output: None,
            }),
        },
    };
    assert_eq!(
        run_inner_with_config(
            command_conversation,
            Config::for_test(root.path().join("data"))
        )
        .await
        .expect("conversation hooks command"),
        0
    );

    let command_correlation = Cli {
        command: Command::Hooks {
            command: HooksCommand::Correlate(CorrelationArgs {
                input: correlation_input,
                json: true,
                output: None,
            }),
        },
    };
    assert_eq!(
        run_inner_with_config(
            command_correlation,
            Config::for_test(root.path().join("data"))
        )
        .await
        .expect("correlation hooks command"),
        0
    );

    let _ = run_hooks_with_config(
        HooksCommand::Shadow(HookShadowArgs {
            event: Some("PreCompact".to_owned()),
            cwd: Some(root.path().to_path_buf()),
            skill_dirs: vec![],
            output: None,
            _jevx_managed: false,
        }),
        Config::for_test(root.path().join("data")),
    )
    .await;

    let _ = run_hooks_with_config(
        HooksCommand::CompactAssist(CompactAssistArgs {
            state_dir: Some(root.path().join("stdin-compaction")),
            _jevx_managed: false,
        }),
        Config::for_test(root.path().join("data")),
    )
    .await;

    let p95 = DistributionSummary {
        mean: Some(1.0),
        stddev: Some(0.0),
        min: Some(1.0),
        max: Some(1.0),
        p50: Some(1.0),
        p95: Some(1.0),
    };
    let mut jevx_modes = BTreeMap::new();
    jevx_modes.insert(
        "jevx".to_owned(),
        RepeatModeSummary {
            status: "completed".to_owned(),
            runs: 1,
            cases_per_run: 1,
            accuracy: p95.clone(),
            none_precision: p95.clone(),
            none_recall: p95.clone(),
            candidate_miss_rate: p95.clone(),
            error_rate: p95.clone(),
            fallback_rate: p95.clone(),
            cache_hit_rate: p95.clone(),
            retry_rate: p95.clone(),
            average_retries: p95.clone(),
            relative_cost: p95.clone(),
            discovery_ms: p95.clone(),
            jev_response_ms: p95.clone(),
            total_ms: p95.clone(),
            input_tokens: p95.clone(),
            output_tokens: p95,
            jev_cost: DistributionSummary {
                mean: Some(1.0),
                stddev: Some(0.0),
                min: Some(1.0),
                max: Some(1.0),
                p50: Some(1.0),
                p95: Some(1.0),
            },
            codex_cost: DistributionSummary {
                mean: None,
                stddev: None,
                min: None,
                max: None,
                p50: None,
                p95: None,
            },
            total_cost: DistributionSummary {
                mean: None,
                stddev: None,
                min: None,
                max: None,
                p50: None,
                p95: None,
            },
        },
    );
    print_repeat_human(&RepeatEvaluationReport {
        schema_version: 2,
        run_count: 1,
        case_count: 1,
        baseline_mode: "local_rank".to_owned(),
        modes: jevx_modes,
        comparisons: BTreeMap::new(),
        runs: Vec::new(),
    });
}

#[tokio::test]
async fn gateway_success_and_new_hook_command_paths_are_exercised() {
    let root = tempdir().expect("tempdir");
    let skill_dir = root.path().join("skills");
    write_skill(&skill_dir, "pdf");

    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let endpoint = format!("http://{}", server.local_addr().expect("address"));
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = server.accept().expect("accept");
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request).expect("request");
        let body = r#"{"answers":{"skill":{"choice":"pdf","probabilities":{"pdf":0.95,"none":0.05}}},"usage":{"inputTokens":12,"outputTokens":3}}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).expect("response");
    });

    let mut gateway_config = Config::for_test(root.path().join("gateway-data"));
    gateway_config.endpoint = endpoint;
    let suggest = SuggestArgs {
        prompt: Some("PDFを確認したい".to_owned()),
        stdin: false,
        input_json: false,
        json: true,
        cwd: Some(root.path().to_path_buf()),
        explicit_skill: None,
        skill_dirs: vec![skill_dir],
        no_telemetry: true,
    };
    assert_eq!(
        run_suggest_with_config(suggest, gateway_config)
            .await
            .expect("gateway"),
        0
    );
    handle.join().expect("server");

    assert_eq!(hook_scope(HookScopeArg::User), HookScope::User);
    assert_eq!(hook_scope(HookScopeArg::Project), HookScope::Project);

    let data_config = Config::for_test(root.path().join("data"));
    assert_eq!(
        run_inner_with_config(
            Cli {
                command: Command::Hooks {
                    command: HooksCommand::Install(HookInstallArgs {
                        scope: HookScopeArg::Project,
                        repo: root.path().to_path_buf(),
                        dry_run: false,
                        allow_content: false,
                        json: false,
                    }),
                },
            },
            data_config.clone(),
        )
        .await
        .expect("install"),
        0
    );
    assert!(root.path().join(".codex/hooks.json").exists());
    assert_eq!(
        run_hook_install(
            HookInstallArgs {
                scope: HookScopeArg::Project,
                repo: root.path().to_path_buf(),
                dry_run: true,
                allow_content: false,
                json: false,
            },
            &data_config,
        )
        .expect("dry run"),
        0
    );
    assert_eq!(
        run_hook_install(
            HookInstallArgs {
                scope: HookScopeArg::Project,
                repo: root.path().to_path_buf(),
                dry_run: true,
                allow_content: false,
                json: true,
            },
            &data_config,
        )
        .expect("json dry run"),
        0
    );

    let stats_input = root.path().join("stats.jsonl");
    fs::write(
    &stats_input,
        r#"{"schemaVersion":1,"promptSha256":"hash","promptChars":3,"decision":"selected","selectedSkill":"pdf","metrics":{"discoveryMs":1,"jevResponseMs":4,"totalMs":5,"candidateCount":1,"inputTokens":2,"outputTokens":1}}"#,
    )
    .expect("stats input");
    assert_eq!(
        run_stats_with_config(true, Some(stats_input), &data_config).expect("stats input"),
        0
    );

    let mut compact_input = Cursor::new(
        br#"{"hook_event_name":"PreCompact","trigger":"manual","cwd":"/tmp"}"#.to_vec(),
    );
    assert_eq!(
        run_compact_assist_from_reader(
            CompactAssistArgs {
                state_dir: Some(root.path().join("reader-compaction")),
                _jevx_managed: false,
            },
            &data_config,
            &mut compact_input,
        )
        .await
        .expect("compact assist"),
        0
    );
}

#[test]
fn input_reader_supports_json_stdin_and_validation_errors() {
    let cwd = PathBuf::from("/tmp/project");
    let json_args = SuggestArgs {
        prompt: None,
        stdin: false,
        input_json: true,
        json: false,
        cwd: None,
        explicit_skill: None,
        skill_dirs: vec![],
        no_telemetry: false,
    };
    let mut json_reader = Cursor::new(br#"{"prompt":"  test  "}"#.to_vec());
    let json_input = read_input_from(&json_args, cwd.clone(), &mut json_reader).expect("json");
    assert_eq!(json_input.prompt, "test");
    assert_eq!(json_input.cwd, PathBuf::from("."));

    let stdin_args = SuggestArgs {
        prompt: None,
        stdin: true,
        input_json: false,
        json: false,
        cwd: Some(cwd.clone()),
        explicit_skill: None,
        skill_dirs: vec![],
        no_telemetry: false,
    };
    let mut prompt_reader = Cursor::new(b"  from stdin  ".to_vec());
    let stdin_input = read_input_from(&stdin_args, cwd.clone(), &mut prompt_reader).expect("stdin");
    assert_eq!(stdin_input.prompt, "from stdin");
    assert_eq!(stdin_input.cwd, cwd);

    let missing_args = SuggestArgs {
        prompt: None,
        stdin: false,
        input_json: false,
        json: false,
        cwd: None,
        explicit_skill: None,
        skill_dirs: vec![],
        no_telemetry: false,
    };
    assert!(matches!(
        read_input_from(&missing_args, PathBuf::from("."), &mut Cursor::new(Vec::new())),
        Err(JevxError::InvalidInput(message)) if message.contains("required")
    ));
    let empty_args = SuggestArgs {
        prompt: Some("  ".to_owned()),
        ..missing_args
    };
    assert!(matches!(
        read_input_from(&empty_args, PathBuf::from("."), &mut Cursor::new(Vec::new())),
        Err(JevxError::InvalidInput(message)) if message.contains("empty")
    ));
}

#[test]
fn helper_functions_cover_roots_and_error_codes() {
    let roots = skill_roots(Path::new("/tmp/project"), &[PathBuf::from("/tmp/extra")]);
    assert_eq!(roots.len(), 5);
    assert_eq!(roots[4].source, "extra");
    assert_eq!(
        skill_roots_with_home(Path::new("/tmp/project"), &[], None, None).len(),
        2
    );

    let json_error = serde_json::from_str::<serde_json::Value>("{").expect_err("json");
    let yaml_error = serde_yaml::from_str::<serde_yaml::Value>("[").expect_err("yaml");
    let errors = vec![
        JevxError::InvalidInput("bad".to_owned()),
        JevxError::MissingApiKey,
        JevxError::Provider("provider".to_owned()),
        JevxError::Timeout,
        JevxError::Io(std::io::Error::other("io")),
        JevxError::Json(json_error),
        JevxError::Yaml(yaml_error),
    ];
    for error in errors {
        let _ = error_code(&error);
        let _ = error_exit_code(&error);
        let _ = error.to_string();
    }
}

#[test]
fn print_human_covers_each_decision() {
    let root = PathBuf::from("/tmp/pdf/SKILL.md");
    let selected = jevx::CandidateResult {
        id: "pdf".to_owned(),
        name: "pdf".to_owned(),
        description: "PDF".to_owned(),
        path: root,
        source: "project".to_owned(),
        local_score: 100,
        probability: Some(0.9),
    };
    for result in [
        SuggestionResult {
            decision: CandidateDecision::Selected,
            selected: Some(selected.clone()),
            ..SuggestionResult::none("selected")
        },
        SuggestionResult {
            decision: CandidateDecision::Explicit,
            selected: Some(selected),
            ..SuggestionResult::none("explicit")
        },
        SuggestionResult {
            decision: CandidateDecision::NoCandidates,
            ..SuggestionResult::none("no_candidates")
        },
        SuggestionResult::none("none_selected"),
        SuggestionResult {
            decision: CandidateDecision::Error,
            ..SuggestionResult::none("error")
        },
        SuggestionResult {
            decision: CandidateDecision::Selected,
            selected: None,
            ..SuggestionResult::none("selected_without_payload")
        },
    ] {
        print_human(&result);
    }
}

#[test]
fn error_run_path_is_represented_by_helper_codes() {
    assert_eq!(error_exit_code(&JevxError::Provider("x".to_owned())), 3);
    assert_eq!(error_exit_code(&JevxError::Timeout), 3);
    assert_eq!(
        error_exit_code(&JevxError::Io(std::io::Error::other("x"))),
        2
    );
    assert_eq!(
        error_exit_code(&JevxError::Yaml(
            serde_yaml::from_str::<serde_yaml::Value>("[").expect_err("yaml")
        )),
        2
    );
}

#[tokio::test]
async fn hooks_uninstall_command_previews_then_removes_only_jevx_handlers() {
    let root = tempdir().expect("tempdir");
    let config = Config::for_test(root.path().join("data"));
    let hooks_path = root.path().join(".codex/hooks.json");
    run_hook_install(
        HookInstallArgs {
            scope: HookScopeArg::Project,
            repo: root.path().to_path_buf(),
            dry_run: false,
            allow_content: false,
            json: true,
        },
        &config,
    )
    .expect("install");
    let installed = fs::read_to_string(&hooks_path).expect("installed");

    let uninstall = |dry_run: bool, json: bool| HookUninstallArgs {
        scope: HookScopeArg::Project,
        repo: root.path().to_path_buf(),
        dry_run,
        json,
    };
    assert_eq!(
        run_hook_uninstall(uninstall(true, true)).expect("preview"),
        0
    );
    assert_eq!(
        run_hook_uninstall(uninstall(true, false)).expect("human preview"),
        0
    );
    assert_eq!(fs::read_to_string(&hooks_path).expect("same"), installed);

    assert_eq!(
        run_inner_with_config(
            Cli {
                command: Command::Hooks {
                    command: HooksCommand::Uninstall(uninstall(false, false)),
                },
            },
            config.clone(),
        )
        .await
        .expect("uninstall"),
        0
    );
    let remaining: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&hooks_path).expect("after")).expect("json");
    assert_eq!(remaining, serde_json::json!({"hooks": {}}));
    assert!(!hooks_path.with_file_name("hooks.json.jevx.bak").exists());

    assert_eq!(
        run_hook_uninstall(uninstall(false, false)).expect("no-op"),
        0
    );
    fs::remove_file(&hooks_path).expect("remove");
    assert_eq!(
        run_hook_uninstall(uninstall(false, true)).expect("missing"),
        0
    );
    assert_eq!(
        run_hook_uninstall(uninstall(false, false)).expect("missing human"),
        0
    );
}

#[tokio::test]
async fn data_commands_show_export_and_purge_only_after_confirmation() {
    let root = tempdir().expect("tempdir");
    let data_home = root.path().join("data");
    fs::create_dir_all(&data_home).expect("data home");
    fs::write(data_home.join("events.jsonl"), "{\"schemaVersion\":1}\n").expect("events");
    let config = Config::for_test(data_home.clone());
    assert_eq!(data_home_of(&config), data_home);

    let run = |command: DataCommand| {
        run_inner_with_config(
            Cli {
                command: Command::Data { command },
            },
            config.clone(),
        )
    };
    assert_eq!(
        run(DataCommand::Path { json: true })
            .await
            .expect("path json"),
        0
    );
    assert_eq!(
        run(DataCommand::Path { json: false }).await.expect("path"),
        0
    );
    assert_eq!(
        run(DataCommand::Export { output: None })
            .await
            .expect("export"),
        0
    );
    let exported = root.path().join("export.json");
    assert_eq!(
        run(DataCommand::Export {
            output: Some(exported.clone())
        })
        .await
        .expect("export file"),
        0
    );
    let exported: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(exported).expect("read")).expect("json");
    assert_eq!(exported["records"]["telemetry"][0]["schemaVersion"], 1);

    for json in [true, false] {
        assert_eq!(
            run(DataCommand::Purge { yes: false, json })
                .await
                .expect("preview"),
            0
        );
    }
    assert!(data_home.join("events.jsonl").exists());
    assert_eq!(
        run(DataCommand::Purge {
            yes: true,
            json: false
        })
        .await
        .expect("purge"),
        0
    );
    assert!(!data_home.join("events.jsonl").exists());
}

fn doctor_env(root: &Path) -> DoctorEnv {
    DoctorEnv {
        home: Some(root.join("home")),
        codex_home: None,
        cwd: root.join("repo"),
        path_var: Some(std::ffi::OsString::from(root.join("bin"))),
    }
}

#[test]
fn doctor_report_suggests_next_steps_until_setup_is_complete() {
    let root = tempdir().expect("tempdir");
    let env = doctor_env(root.path());
    let mut config = Config::for_test(root.path().join("data"));
    config.api_key = None;
    config.warnings = vec!["JEVX_MIN_MARGIN must be between 0 and 1; using default 0.1".to_owned()];

    let report = doctor_report(&config, &env);
    assert_eq!(report["schemaVersion"], 2);
    assert_eq!(report["apiKeyConfigured"], false);
    assert_eq!(report["jevxOnPath"], false);
    assert_eq!(report["skillInstalled"], false);
    assert_eq!(
        report["hooks"]["user"]["installedHandlers"],
        serde_json::Value::Null
    );
    assert_eq!(report["thresholds"]["minProbability"], 0.6);
    assert_eq!(report["thresholds"]["customized"], false);
    assert_eq!(report["warnings"][0], config.warnings[0]);
    let steps = report["nextSteps"].as_array().expect("steps");
    let text = serde_json::to_string(steps).expect("text");
    assert!(text.contains("setup.sh --scope user"));
    assert!(text.contains("AI_GATEWAY_API_KEY"));
    assert!(text.contains("PATH"));

    fs::create_dir_all(root.path().join("bin")).expect("bin");
    fs::write(root.path().join("bin/jevx"), "").expect("jevx");
    fs::create_dir_all(root.path().join("home/.agents/skills/jevx")).expect("skill");
    fs::write(
        root.path().join("home/.agents/skills/jevx/SKILL.md"),
        "---\nname: jevx\n---\n",
    )
    .expect("skill file");
    fs::create_dir_all(root.path().join("home/.codex")).expect("codex");
    fs::write(
        root.path().join("home/.codex/hooks.json"),
        r#"{"hooks":{"UserPromptSubmit":[{"hooks":[{"type":"command","command":"'/bin/jevx' hooks shadow --jevx-managed"}]}]}}"#,
    )
    .expect("hooks");
    fs::create_dir_all(root.path().join("repo/.codex")).expect("project codex");
    fs::write(root.path().join("repo/.codex/hooks.json"), "not json").expect("invalid hooks");
    let ready = doctor_report(&Config::for_test(root.path().join("data")), &env);
    assert_eq!(ready["jevxOnPath"], true);
    assert_eq!(ready["skillInstalled"], true);
    assert_eq!(ready["hooks"]["user"]["installedHandlers"], 1);
    assert!(ready["hooks"]["project"]["error"].is_string());
    let steps = serde_json::to_string(&ready["nextSteps"]).expect("steps");
    assert!(steps.contains("skills suggest"));
    assert!(!steps.contains("setup.sh"));
    assert!(!steps.contains("Optional"));

    fs::remove_file(root.path().join("home/.codex/hooks.json")).expect("remove user hooks");
    fs::remove_file(root.path().join("repo/.codex/hooks.json")).expect("remove project hooks");
    let without_hooks = doctor_report(&Config::for_test(root.path().join("data")), &env);
    let steps = serde_json::to_string(&without_hooks["nextSteps"]).expect("steps");
    assert!(steps.contains("Optional: preview Codex hooks"));
}

#[test]
fn doctor_command_prints_json_and_human_reports() {
    let root = tempdir().expect("tempdir");
    let config = Config::for_test(root.path().join("data"));
    assert_eq!(run_doctor_with_config(true, &config).expect("json"), 0);
    assert_eq!(run_doctor_with_config(false, &config).expect("human"), 0);
    let human = doctor_human_output(&doctor_report(&config, &doctor_env(root.path())));
    assert!(human.contains("Next steps:"));
    assert!(human.contains("Thresholds: minProbability=0.6"));

    let mut warned = config.clone();
    warned.warnings = vec!["JEVX_MIN_MARGIN must be between 0 and 1".to_owned()];
    let mut report = doctor_report(&warned, &doctor_env(root.path()));
    report["endpoint"] = serde_json::json!(null);
    let human = doctor_human_output(&report);
    assert!(human.contains("Warning: JEVX_MIN_MARGIN"));
    assert!(human.contains("Endpoint: null"));
}

#[test]
fn eval_skill_roots_use_only_the_explicit_catalog() {
    let root = tempdir().expect("tempdir");
    let roots = eval_skill_roots(&[root.path().join("catalog")]);
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].path, root.path().join("catalog"));
}
