//! `jevx skills suggest` / `jevx skills list`: Skill探索とJev判定の入口。

use std::env;
use std::io::{self, Read};
use std::path::PathBuf;

use jevx::{
    CandidateDecision, Config, GatewayJudge, JevxError, SuggestInput, SuggestionResult,
    TelemetryEvent, append_telemetry, discover_skill_roots, suggest_with_judge,
    suggest_with_optional_judge,
};

use super::*;

pub(super) async fn run_suggest_with_config(
    args: SuggestArgs,
    config: Config,
) -> Result<i32, JevxError> {
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
    let mut execution_config = config.clone();
    if args.no_telemetry {
        execution_config.telemetry_enabled = false;
    }

    let result = if input.explicit_skill.is_some() {
        suggest_with_optional_judge::<GatewayJudge>(input.clone(), skills, &execution_config, None)
            .await
    } else {
        match GatewayJudge::from_config(&execution_config) {
            Ok(judge) => suggest_with_judge(input.clone(), skills, &execution_config, &judge).await,
            Err(JevxError::MissingApiKey) => {
                suggest_with_optional_judge::<GatewayJudge>(
                    input.clone(),
                    skills,
                    &execution_config,
                    None,
                )
                .await
            }
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

pub(super) fn run_list(args: ListArgs) -> Result<i32, JevxError> {
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

pub(super) fn read_input(args: &SuggestArgs, cwd: PathBuf) -> Result<SuggestInput, JevxError> {
    let mut stdin = io::stdin();
    read_input_from(args, cwd, &mut stdin)
}

pub(super) fn read_input_from<R: Read>(
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

pub(super) fn print_human(result: &SuggestionResult) {
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
