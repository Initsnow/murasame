use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub metadata: SkillMetadata,
    pub instructions: String,
    pub dir_path: PathBuf,
    pub file_path: PathBuf,
    pub is_character_specific: bool,
}

impl Skill {
    pub fn parse_from_str(
        content: &str,
        dir_path: PathBuf,
        file_path: PathBuf,
        is_character_specific: bool,
    ) -> Result<Self> {
        let trimmed = content.trim_start();
        if !trimmed.starts_with("---") {
            return Err(anyhow!("Missing YAML frontmatter starting with '---' in SKILL.md"));
        }

        let after_start = &trimmed[3..];
        let next_delim = after_start
            .find("\n---")
            .or_else(|| after_start.find("\r\n---"))
            .ok_or_else(|| anyhow!("Unclosed YAML frontmatter in SKILL.md"))?;

        let yaml_str = &after_start[..next_delim];
        let remainder = &after_start[next_delim..];

        // Strip closing --- and optional newline
        let instructions = if let Some(stripped) = remainder.strip_prefix("\n---") {
            stripped.trim_start_matches(|c| c == '\n' || c == '\r').trim()
        } else if let Some(stripped) = remainder.strip_prefix("\r\n---") {
            stripped.trim_start_matches(|c| c == '\n' || c == '\r').trim()
        } else {
            remainder.trim()
        };

        let metadata: SkillMetadata = serde_yaml::from_str(yaml_str)
            .context("Failed to deserialize YAML frontmatter into SkillMetadata")?;

        if metadata.name.trim().is_empty() {
            return Err(anyhow!("Skill name in metadata cannot be empty"));
        }

        Ok(Self {
            metadata,
            instructions: instructions.to_string(),
            dir_path,
            file_path,
            is_character_specific,
        })
    }

    pub fn load_file(path: impl AsRef<Path>, is_character_specific: bool) -> Result<Self> {
        let file_path = path.as_ref().to_path_buf();
        let dir_path = file_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let content = std::fs::read_to_string(&file_path)
            .with_context(|| format!("Failed to read skill file: {}", file_path.display()))?;
        Self::parse_from_str(&content, dir_path, file_path, is_character_specific)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_skill_from_str() {
        let raw = r#"---
name: web_search
description: "Search current information using web query"
version: "1.0.0"
---
# Web Search Guide
1. Identify keywords.
2. Call search API.
"#;
        let skill = Skill::parse_from_str(
            raw,
            PathBuf::from("skills/web_search"),
            PathBuf::from("skills/web_search/SKILL.md"),
            false,
        )
        .unwrap();

        assert_eq!(skill.metadata.name, "web_search");
        assert_eq!(
            skill.metadata.description,
            "Search current information using web query"
        );
        assert_eq!(skill.metadata.version.as_deref(), Some("1.0.0"));
        assert!(skill.instructions.contains("# Web Search Guide"));
        assert!(!skill.is_character_specific);
    }

    #[test]
    fn test_invalid_frontmatter() {
        let invalid = "Just some markdown without frontmatter";
        let res = Skill::parse_from_str(
            invalid,
            PathBuf::from("."),
            PathBuf::from("SKILL.md"),
            false,
        );
        assert!(res.is_err());
    }
}
