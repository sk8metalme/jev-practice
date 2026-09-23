//! `jevx data path|export|purge`: jevxが保存したローカルデータの確認・持ち出し・削除。

use std::fs;

use jevx::data::{data_inventory, export_data, purge_data};
use jevx::{Config, JevxError};

use super::*;

pub(super) fn run_data_with_config(
    command: DataCommand,
    config: &Config,
) -> Result<i32, JevxError> {
    let inventory = data_inventory(&data_home_of(config))?;
    match command {
        DataCommand::Path { json } => {
            if json {
                println!("{}", serde_json::to_string_pretty(&inventory)?);
            } else {
                println!("Data home: {}", inventory.data_home.display());
                for file in &inventory.files {
                    let state = if file.exists {
                        format!("{} records, {} bytes", file.records, file.bytes)
                    } else {
                        "not created".to_owned()
                    };
                    println!("  {:<22} {} ({state})", file.kind, file.path.display());
                }
            }
        }
        DataCommand::Export { output } => {
            let mut content = serde_json::to_string_pretty(&export_data(&inventory)?)?;
            content.push('\n');
            match output {
                Some(path) => fs::write(path, content)?,
                None => print!("{content}"),
            }
        }
        DataCommand::Purge { yes, json } => {
            let report = purge_data(&inventory, yes)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                let verb = if report.dry_run {
                    "Would delete"
                } else {
                    "Deleted"
                };
                for path in &report.removed {
                    println!("{verb} {}", path.display());
                }
                println!("{verb} {} file(s)", report.removed.len());
                if report.dry_run && !report.removed.is_empty() {
                    println!("Run `jevx data purge --yes` to delete them.");
                }
            }
        }
    }
    Ok(0)
}
