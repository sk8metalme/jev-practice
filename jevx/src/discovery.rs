use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::JevxError;
use crate::types::SkillRecord;

#[derive(Debug, Clone)]
pub struct SkillRoot {
    pub path: PathBuf,
    pub source: String,
    priority: usize,
}

impl SkillRoot {
    pub fn new(path: PathBuf, source: String, priority: usize) -> Self {
        Self {
            path,
            source,
            priority,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
}

pub fn parse_skill_file(path: &Path, source: &str) -> Result<Option<SkillRecord>, JevxError> {
    let content = fs::read_to_string(path)?;
    let Some(frontmatter) = extract_frontmatter(&content) else {
        return Ok(None);
    };
    let parsed: Frontmatter = serde_yaml::from_str(frontmatter)?;
    let Some(name) = parsed
        .name
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let Some(description) = parsed
        .description
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    Ok(Some(SkillRecord::new(
        name.clone(),
        description,
        path.to_path_buf(),
        source.to_owned(),
    )))
}

pub fn discover_skills(roots: &[PathBuf]) -> Result<Vec<SkillRecord>, JevxError> {
    let roots = roots
        .iter()
        .enumerate()
        .map(|(priority, path)| SkillRoot {
            path: path.clone(),
            source: "project".to_owned(),
            priority,
        })
        .collect::<Vec<_>>();
    discover_skill_roots(&roots)
}

pub fn discover_skill_roots(roots: &[SkillRoot]) -> Result<Vec<SkillRecord>, JevxError> {
    let mut found: HashMap<String, (usize, SkillRecord)> = HashMap::new();
    for root in roots {
        if !root.path.exists() {
            continue;
        }
        let mut files = Vec::new();
        collect_skill_files(&root.path, &mut files)?;
        for path in files {
            if let Ok(Some(skill)) = parse_skill_file(&path, &root.source) {
                let key = skill.id.to_ascii_lowercase();
                let should_replace = found
                    .get(&key)
                    .map(|(priority, _)| root.priority < *priority)
                    .unwrap_or(true);
                if should_replace {
                    found.insert(key, (root.priority, skill));
                }
            }
        }
    }

    let mut skills = found
        .into_values()
        .map(|(_, skill)| skill)
        .collect::<Vec<_>>();
    skills.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });
    Ok(skills)
}

fn collect_skill_files(path: &Path, output: &mut Vec<PathBuf>) -> Result<(), JevxError> {
    if path.is_file() {
        if path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md") {
            output.push(path.to_path_buf());
        }
        return Ok(());
    }

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        if entry_path.is_dir()
            && entry_path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with('.') || name == "target")
        {
            continue;
        }
        collect_skill_files(&entry_path, output)?;
    }
    Ok(())
}

fn extract_frontmatter(content: &str) -> Option<&str> {
    let content = content.strip_prefix("---\n")?;
    let end = content.find("\n---")?;
    Some(&content[..end])
}
