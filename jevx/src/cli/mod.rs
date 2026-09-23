//! CLIの引数定義とdispatch。各コマンドの処理は同名のサブモジュールにある。

use std::env;
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand, ValueEnum};

use jevx::{Config, JevxError, SkillRoot};

mod data;
mod doctor;
mod eval;
mod hooks;
mod skills;
mod stats;

use self::data::*;
use self::doctor::*;
use self::eval::*;
use self::hooks::*;
use self::skills::*;
use self::stats::*;

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
        #[arg(long)]
        input: Option<PathBuf>,
    },
    Doctor {
        #[arg(long)]
        json: bool,
    },
    Eval(EvalArgs),
    EvalRepeat(EvalRepeatArgs),
    Hooks {
        #[command(subcommand)]
        command: HooksCommand,
    },
    /// Show, export, or purge the local data jevx has written under $JEVX_HOME.
    Data {
        #[command(subcommand)]
        command: DataCommand,
    },
}

#[derive(Debug, Subcommand)]
enum DataCommand {
    /// List managed files with their sizes and record counts.
    Path {
        #[arg(long)]
        json: bool,
    },
    /// Print every stored record as one JSON document.
    Export {
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Delete managed files. Without --yes, only lists what would be deleted.
    Purge {
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        json: bool,
    },
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

#[derive(Debug, Args)]
struct EvalRepeatArgs {
    #[arg(long, default_value = "5")]
    runs: usize,
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

#[derive(Debug, Subcommand)]
enum HooksCommand {
    Shadow(HookShadowArgs),
    Install(HookInstallArgs),
    Uninstall(HookUninstallArgs),
    CompactAssist(CompactAssistArgs),
    CompactEval(CompactEvalArgs),
    ConversationEval(ConversationEvalArgs),
    Correlate(CorrelationArgs),
}

#[derive(Debug, Args)]
struct HookShadowArgs {
    #[arg(long)]
    event: Option<String>,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long = "skill-dir")]
    skill_dirs: Vec<PathBuf>,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long = "jevx-managed", hide = true)]
    _jevx_managed: bool,
}

#[derive(Debug, Clone, ValueEnum)]
enum HookScopeArg {
    User,
    Project,
}

#[derive(Debug, Args)]
struct HookInstallArgs {
    #[arg(long, value_enum, default_value = "user")]
    scope: HookScopeArg,
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct HookUninstallArgs {
    #[arg(long, value_enum, default_value = "user")]
    scope: HookScopeArg,
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct CompactAssistArgs {
    #[arg(long)]
    state_dir: Option<PathBuf>,
    #[arg(long = "jevx-managed", hide = true)]
    _jevx_managed: bool,
}

#[derive(Debug, Args)]
struct CompactEvalArgs {
    #[arg(long, default_value = "5")]
    runs: usize,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ConversationEvalArgs {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct CorrelationArgs {
    #[arg(long)]
    input: PathBuf,
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

pub(super) async fn run_inner(cli: Cli) -> Result<i32, JevxError> {
    run_inner_with_config(cli, Config::from_env()).await
}

pub(super) async fn run_inner_with_config(cli: Cli, config: Config) -> Result<i32, JevxError> {
    match cli.command {
        Command::Skills { command } => match command {
            SkillsCommand::Suggest(args) => run_suggest_with_config(args, config).await,
            SkillsCommand::List(args) => run_list(args),
        },
        Command::Stats { json, input } => run_stats_with_config(json, input, &config),
        Command::Doctor { json } => run_doctor_with_config(json, &config),
        Command::Eval(args) => run_eval_with_config(args, config).await,
        Command::EvalRepeat(args) => run_eval_repeat_with_config(args, config).await,
        Command::Hooks { command } => run_hooks_with_config(command, config).await,
        Command::Data { command } => run_data_with_config(command, &config),
    }
}

/// jevxがローカルデータを書き込むディレクトリ（`$JEVX_HOME`、既定は `~/.jevx`）。
pub(super) fn data_home_of(config: &Config) -> PathBuf {
    config
        .telemetry_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub(super) fn skill_roots(cwd: &Path, extra: &[PathBuf]) -> Vec<SkillRoot> {
    skill_roots_with_home(
        cwd,
        extra,
        env::var_os("HOME").map(PathBuf::from),
        env::var_os("CODEX_HOME").map(PathBuf::from),
    )
}

pub(super) fn skill_roots_with_home(
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

pub(super) fn format_ratio(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.3}"))
        .unwrap_or_else(|| "n/a".to_owned())
}

pub(super) fn format_optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_owned())
}

pub(super) fn format_optional_f64(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.1}"))
        .unwrap_or_else(|| "n/a".to_owned())
}

pub(super) fn error_code(error: &JevxError) -> &'static str {
    match error {
        JevxError::InvalidInput(_) => "invalid_input",
        JevxError::MissingApiKey => "missing_api_key",
        JevxError::Provider(_) => "provider_error",
        JevxError::ProviderWithMetrics { .. } => "provider_error",
        JevxError::Timeout => "timeout",
        JevxError::Io(_) => "io_error",
        JevxError::Json(_) => "json_error",
        JevxError::Yaml(_) => "yaml_error",
    }
}

pub(super) fn error_exit_code(error: &JevxError) -> i32 {
    match error {
        JevxError::InvalidInput(_) | JevxError::MissingApiKey | JevxError::Json(_) => 2,
        JevxError::Provider(_) | JevxError::ProviderWithMetrics { .. } | JevxError::Timeout => 3,
        JevxError::Io(_) | JevxError::Yaml(_) => 2,
    }
}

#[cfg(test)]
mod tests;
