use std::env;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};

use jevx::evaluation::{EvaluationReport, evaluate, load_fixtures, write_case_results};
use jevx::{
    CandidateDecision, Config, GatewayJudge, JevxError, SkillRoot, SuggestInput, SuggestionResult,
    TelemetryEvent, append_telemetry, discover_skill_roots, read_stats, suggest_with_judge,
    suggest_with_optional_judge,
};

#[derive(Debug, Parser)]
#[command(
    name = "jevx",
    version,
    about = "Jev-powered comfort layer for Codex CLI"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Skills {
        #[command(subcommand)]
        command: SkillsCommand,
    },
    Stats {
        #[arg(long)]
        json: bool,
    },
    Doctor {
        #[arg(long)]
        json: bool,
    },
    Eval(EvalArgs),
}

#[derive(Debug, Subcommand)]
enum SkillsCommand {
    Suggest(SuggestArgs),
    List(ListArgs),
}

#[derive(Debug, Args)]
struct SuggestArgs {
    #[arg(long, conflicts_with_all = ["stdin", "input_json"])]
    prompt: Option<String>,
    #[arg(long, conflicts_with_all = ["prompt", "input_json"])]
    stdin: bool,
    #[arg(long = "input-json", conflicts_with_all = ["prompt", "stdin"])]
    input_json: bool,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long = "skill")]
    explicit_skill: Option<String>,
    #[arg(long = "skill-dir")]
    skill_dirs: Vec<PathBuf>,
    #[arg(long)]
    no_telemetry: bool,
}

#[derive(Debug, Args)]
struct ListArgs {
    #[arg(long)]
    json: bool,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long = "skill-dir")]
    skill_dirs: Vec<PathBuf>,
}

#[derive(Debug, Args)]
struct EvalArgs {
    #[arg(long, default_value = "jevx/evals/skill-selection.jsonl")]
    fixtures: PathBuf,
    #[arg(long = "skill-dir", default_value = "jevx/evals/skills")]
    skill_dirs: Vec<PathBuf>,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    output: Option<PathBuf>,
}

pub async fn run_with_cli(cli: Cli) -> i32 {
    match run_inner(cli).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("jevx: {error}");
            error_exit_code(&error)
        }
    }
}

async fn run_inner(cli: Cli) -> Result<i32, JevxError> {
    run_inner_with_config(cli, Config::from_env()).await
}

async fn run_inner_with_config(cli: Cli, config: Config) -> Result<i32, JevxError> {
    match cli.command {
        Command::Skills { command } => match command {
            SkillsCommand::Suggest(args) => run_suggest_with_config(args, config).await,
            SkillsCommand::List(args) => run_list(args),
        },
        Command::Stats { json } => run_stats_with_config(json, &config),
        Command::Doctor { json } => run_doctor_with_config(json, &config),
        Command::Eval(args) => run_eval_with_config(args, config).await,
    }
}

async fn run_suggest_with_config(args: SuggestArgs, config: Config) -> Result<i32, JevxError> {
    let cwd = args
        .cwd
        .clone()
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let mut input = read_input(&args, cwd.clone())?;
    if let Some(explicit_skill) = args.explicit_skill {
        input.explicit_skill = Some(explicit_skill);
    }
    let roots = skill_roots(&cwd, &args.skill_dirs);
    let skills = discover_skill_roots(&roots)?;

    let result = if input.explicit_skill.is_some() {
        suggest_with_optional_judge::<GatewayJudge>(input.clone(), skills, &config, None).await
    } else {
        match GatewayJudge::from_config(&config) {
            Ok(judge) => suggest_with_judge(input.clone(), skills, &config, &judge).await,
            Err(error) => Err(error),
        }
    };
    match result {
        Ok(result) => {
            if config.telemetry_enabled && !args.no_telemetry {
                let event = TelemetryEvent::from_result(
                    &input.prompt,
                    &result.decision,
                    result.selected.as_ref(),
                    &result.metrics,
                );
                append_telemetry(&config.telemetry_path, &event)?;
            }
            if args.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print_human(&result);
            }
            Ok(0)
        }
        Err(error) => {
            if config.telemetry_enabled && !args.no_telemetry {
                let event = TelemetryEvent::from_result(
                    &input.prompt,
                    &CandidateDecision::Error,
                    None,
                    &jevx::Metrics::default(),
                );
                append_telemetry(&config.telemetry_path, &event)?;
            }
            if args.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "schemaVersion": 1,
                        "decision": "error",
                        "error": { "code": error_code(&error), "message": error.to_string() }
                    })
                );
                Ok(error_exit_code(&error))
            } else {
                Err(error)
            }
        }
    }
}

fn run_list(args: ListArgs) -> Result<i32, JevxError> {
    let cwd = args
        .cwd
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let skills = discover_skill_roots(&skill_roots(&cwd, &args.skill_dirs))?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&skills)?);
    } else {
        for skill in skills {
            println!("{}\t{}\t{}", skill.name, skill.source, skill.path.display());
            println!("  {}", skill.description);
        }
    }
    Ok(0)
}

async fn run_eval_with_config(mut args: EvalArgs, mut config: Config) -> Result<i32, JevxError> {
    let cwd = args
        .cwd
        .take()
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let fixtures = load_fixtures(&args.fixtures)?;
    let skills = discover_skill_roots(&skill_roots(&cwd, &args.skill_dirs))?;
    config.telemetry_enabled = false;

    let report = if args.dry_run {
        evaluate(&fixtures, &skills, &config, None).await
    } else {
        let judge = GatewayJudge::from_config(&config)?;
        evaluate(&fixtures, &skills, &config, Some(&judge)).await
    };
    if let Some(output) = args.output {
        write_case_results(&output, &report.cases)?;
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_evaluation_human(&report);
    }
    Ok(0)
}

fn run_stats_with_config(json: bool, config: &Config) -> Result<i32, JevxError> {
    let stats = read_stats(&config.telemetry_path)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        println!("events: {}", stats.events);
        println!("selected: {}", stats.selected);
        println!("none: {}", stats.none);
        println!("errors: {}", stats.errors);
        println!(
            "average Jev response: {:.1} ms",
            stats.average_jev_response_ms
        );
    }
    Ok(0)
}

fn run_doctor_with_config(json: bool, config: &Config) -> Result<i32, JevxError> {
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

fn read_input(args: &SuggestArgs, cwd: PathBuf) -> Result<SuggestInput, JevxError> {
    let mut stdin = io::stdin();
    read_input_from(args, cwd, &mut stdin)
}

fn read_input_from<R: Read>(
    args: &SuggestArgs,
    cwd: PathBuf,
    reader: &mut R,
) -> Result<SuggestInput, JevxError> {
    if args.input_json {
        let mut body = String::new();
        reader.read_to_string(&mut body)?;
        let mut input = SuggestInput::from_json(&body)?;
        input.cwd = args.cwd.clone().unwrap_or(input.cwd);
        return Ok(input);
    }
    let prompt = if let Some(prompt) = &args.prompt {
        prompt.clone()
    } else if args.stdin {
        let mut prompt = String::new();
        reader.read_to_string(&mut prompt)?;
        prompt
    } else {
        return Err(JevxError::InvalidInput(
            "one of --prompt, --stdin, or --input-json is required".to_owned(),
        ));
    };
    let prompt = prompt.trim().to_owned();
    if prompt.is_empty() {
        return Err(JevxError::InvalidInput(
            "prompt must not be empty".to_owned(),
        ));
    }
    Ok(SuggestInput::new(prompt, cwd))
}

fn skill_roots(cwd: &Path, extra: &[PathBuf]) -> Vec<SkillRoot> {
    skill_roots_with_home(
        cwd,
        extra,
        env::var_os("HOME").map(PathBuf::from),
        env::var_os("CODEX_HOME").map(PathBuf::from),
    )
}

fn skill_roots_with_home(
    cwd: &Path,
    extra: &[PathBuf],
    home: Option<PathBuf>,
    codex_home: Option<PathBuf>,
) -> Vec<SkillRoot> {
    let mut roots = vec![
        SkillRoot::new(cwd.join(".agents/skills"), "project".to_owned(), 0),
        SkillRoot::new(cwd.join(".codex/skills"), "project".to_owned(), 1),
    ];
    if let Some(home) = home {
        roots.push(SkillRoot::new(
            home.join(".agents/skills"),
            "user".to_owned(),
            2,
        ));
        let codex_home = codex_home.unwrap_or_else(|| home.join(".codex"));
        roots.push(SkillRoot::new(
            codex_home.join("skills"),
            "user".to_owned(),
            3,
        ));
    }
    for (offset, path) in extra.iter().enumerate() {
        roots.push(SkillRoot::new(
            path.clone(),
            "extra".to_owned(),
            10 + offset,
        ));
    }
    roots
}

fn print_human(result: &SuggestionResult) {
    println!("jevx skill suggestion");
    match result.decision {
        CandidateDecision::Selected | CandidateDecision::Explicit => {
            if let Some(selected) = &result.selected {
                println!("Selected: {}", selected.name);
                if let Some(probability) = selected.probability {
                    println!("Probability: {:.2}", probability);
                }
                println!("Path: {}", selected.path.display());
            }
        }
        CandidateDecision::NoCandidates => println!("Selected: none (no candidates)"),
        CandidateDecision::None => println!(
            "Selected: none ({})",
            result.reason_code.as_deref().unwrap_or("not recommended")
        ),
        CandidateDecision::Error => println!("Selected: none (error)"),
    }
    println!("Candidates: {}", result.metrics.candidate_count);
    println!("Jev response: {} ms", result.metrics.jev_response_ms);
    println!("Total: {} ms", result.metrics.total_ms);
    println!("Mode: {}", result.mode);
}

fn print_evaluation_human(report: &EvaluationReport) {
    println!("jevx evaluation");
    println!("Cases: {}", report.case_count);
    for (mode, summary) in &report.modes {
        println!(
            "{mode}: status={} cases={} accuracy={} nonePrecision={} errorRate={}",
            summary.status,
            summary.cases,
            format_ratio(summary.accuracy),
            format_ratio(summary.none_precision),
            format_ratio(summary.error_rate),
        );
        if let Some(response_ms) = summary.jev_response_ms_p50 {
            println!(
                "  Jev response: p50={response_ms} ms p95={} ms",
                summary.jev_response_ms_p95.unwrap_or(response_ms)
            );
        }
        if let Some(total_ms) = summary.total_ms_p50 {
            println!(
                "  Total: p50={total_ms} ms p95={} ms",
                summary.total_ms_p95.unwrap_or(total_ms)
            );
        }
    }
}

fn format_ratio(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.3}"))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn error_code(error: &JevxError) -> &'static str {
    match error {
        JevxError::InvalidInput(_) => "invalid_input",
        JevxError::MissingApiKey => "missing_api_key",
        JevxError::Provider(_) => "provider_error",
        JevxError::Timeout => "timeout",
        JevxError::Io(_) => "io_error",
        JevxError::Json(_) => "json_error",
        JevxError::Yaml(_) => "yaml_error",
    }
}

fn error_exit_code(error: &JevxError) -> i32 {
    match error {
        JevxError::InvalidInput(_) | JevxError::MissingApiKey | JevxError::Json(_) => 2,
        JevxError::Provider(_) | JevxError::Timeout => 3,
        JevxError::Io(_) | JevxError::Yaml(_) => 2,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::Cursor;
    use std::time::Duration;

    use super::*;
    use jevx::evaluation::{EvaluationReport, ModeSummary};
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
                command: Command::Stats { json: true },
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

        let stats_json = run_stats_with_config(true, &config).expect("stats");
        assert_eq!(stats_json, 0);
        let stats_human = run_stats_with_config(false, &config).expect("stats");
        assert_eq!(stats_human, 0);
        let doctor_json = run_doctor_with_config(true, &config).expect("doctor");
        assert_eq!(doctor_json, 0);
        let doctor_human = run_doctor_with_config(false, &config).expect("doctor");
        assert_eq!(doctor_human, 0);
        assert_eq!(
            run_inner_with_config(
                Cli {
                    command: Command::Stats { json: true },
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
                candidate_misses: 0,
                candidate_miss_rate: Some(0.0),
                errors: 0,
                error_rate: Some(0.0),
                jev_response_ms_p50: Some(12),
                jev_response_ms_p95: Some(18),
                total_ms_p50: Some(15),
                total_ms_p95: Some(22),
                average_input_tokens: Some(8.0),
                average_output_tokens: Some(2.0),
            },
        );
        print_evaluation_human(&EvaluationReport {
            schema_version: 1,
            case_count: 1,
            modes,
            cases: Vec::new(),
        });
        assert_eq!(format_ratio(Some(0.5)), "0.500");
        assert_eq!(format_ratio(None), "n/a");
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
        let stdin_input =
            read_input_from(&stdin_args, cwd.clone(), &mut prompt_reader).expect("stdin");
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
}
