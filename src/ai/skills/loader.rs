use std::{
    collections::HashMap,
    path::Path,
};

use super::model::Skill;

#[derive(Debug, Clone, Default)]
pub struct SkillManager {
    skills: HashMap<String, Skill>,
}

impl SkillManager {
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
        }
    }

    /// Load skills from global skills directory and character-specific directory
    pub fn load_all(
        global_skills_dir: impl AsRef<Path>,
        character_id: Option<&str>,
        characters_base_dir: impl AsRef<Path>,
    ) -> Self {
        let mut mgr = Self::new();

        // 1. Scan global skills
        let global_path = global_skills_dir.as_ref();
        if global_path.is_dir() {
            mgr.scan_directory(global_path, false);
        }

        // 2. Scan character specific skills
        if let Some(cid) = character_id {
            let char_skills_path = characters_base_dir.as_ref().join(cid).join("skills");
            if char_skills_path.is_dir() {
                mgr.scan_directory(&char_skills_path, true);
            }
        }

        mgr
    }

    fn scan_directory(&mut self, base_dir: &Path, is_character: bool) {
        let entries = match std::fs::read_dir(base_dir) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Failed to read skills directory {}: {e}", base_dir.display());
                return;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Look for SKILL.md inside subdirectory
                let skill_file = path.join("SKILL.md");
                if skill_file.is_file() {
                    match Skill::load_file(&skill_file, is_character) {
                        Ok(skill) => {
                            self.skills.insert(skill.metadata.name.clone(), skill);
                        }
                        Err(e) => {
                            tracing::warn!("Failed to parse skill at {}: {e}", skill_file.display());
                        }
                    }
                }
            } else if path.is_file() && path.file_name().map(|n| n == "SKILL.md").unwrap_or(false) {
                // Directly a SKILL.md in the scanned directory
                if let Ok(skill) = Skill::load_file(&path, is_character) {
                    self.skills.insert(skill.metadata.name.clone(), skill);
                }
            }
        }
    }

    pub fn get_skill(&self, name: &str) -> Option<&Skill> {
        self.skills.get(name)
    }

    pub fn list_skills(&self) -> Vec<&Skill> {
        let mut list: Vec<&Skill> = self.skills.values().collect();
        list.sort_by(|a, b| a.metadata.name.cmp(&b.metadata.name));
        list
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    pub fn len(&self) -> usize {
        self.skills.len()
    }

    pub fn reload(
        &mut self,
        global_skills_dir: impl AsRef<Path>,
        character_id: Option<&str>,
        characters_base_dir: impl AsRef<Path>,
    ) {
        *self = Self::load_all(global_skills_dir, character_id, characters_base_dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_scan_and_override() {
        let temp_dir = std::env::temp_dir().join(format!("murasame_test_skills_{}", uuid::Uuid::new_v4()));
        let global_dir = temp_dir.join("skills");
        let char_base = temp_dir.join("characters");
        let char_dir = char_base.join("murasame").join("skills");

        fs::create_dir_all(global_dir.join("echo")).unwrap();
        fs::create_dir_all(char_dir.join("echo")).unwrap();

        let global_skill = r#"---
name: echo
description: "Global echo skill"
---
Instructions from global
"#;
        fs::write(global_dir.join("echo").join("SKILL.md"), global_skill).unwrap();

        let char_skill = r#"---
name: echo
description: "Character specific echo skill"
---
Instructions from character
"#;
        fs::write(char_dir.join("echo").join("SKILL.md"), char_skill).unwrap();

        let mgr = SkillManager::load_all(&global_dir, Some("murasame"), &char_base);
        let loaded = mgr.get_skill("echo").expect("Skill echo must be present");

        assert_eq!(loaded.metadata.description, "Character specific echo skill");
        assert!(loaded.is_character_specific);
        assert!(loaded.instructions.contains("Instructions from character"));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
