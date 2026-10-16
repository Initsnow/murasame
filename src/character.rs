use crate::config::{AiConfig, AudioConfig, CueConfig, InteractionConfig, ModelConfig};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

fn default_avatar_path() -> String {
    "avatar.png".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterConfig {
    pub id: String,
    pub name: String,
    #[serde(default = "default_avatar_path")]
    pub avatar_path: String,
    #[serde(default)]
    pub model: ModelConfig,
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub ai: AiConfig,
    #[serde(default)]
    pub cues: CueConfig,
    #[serde(default)]
    pub interactions: InteractionConfig,
}

impl CharacterConfig {
    pub fn default_for(id: &str, name: &str) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            avatar_path: default_avatar_path(),
            model: ModelConfig::default(),
            audio: AudioConfig::default(),
            ai: AiConfig::default(),
            cues: CueConfig::default(),
            interactions: InteractionConfig::default(),
        }
    }

    pub fn clone_with_new_id(&self, new_id: &str, new_name: &str) -> Self {
        let mut cloned = self.clone();
        cloned.id = new_id.to_string();
        cloned.name = new_name.to_string();
        cloned
    }

    pub fn get_avatar_full_path(&self, base_dir: impl AsRef<Path>) -> PathBuf {
        let path = Path::new(&self.avatar_path);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            base_dir.as_ref().join(&self.id).join(path)
        }
    }

    pub fn save_to_disk(&self, base_dir: impl AsRef<Path>) -> Result<()> {
        let char_dir = base_dir.as_ref().join(&self.id);
        fs::create_dir_all(&char_dir)?;
        let toml_file = char_dir.join("character.toml");
        let content =
            toml::to_string_pretty(self).context("Failed to serialize character config")?;
        fs::write(toml_file, content)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct CharacterSummary {
    pub id: String,
    pub name: String,
    pub avatar_path: PathBuf,
    pub is_active: bool,
}

pub struct CharacterManager {
    pub active_id: String,
    pub characters: BTreeMap<String, CharacterConfig>,
    pub base_dir: PathBuf,
}

impl CharacterManager {
    pub fn load_all(base_dir: impl AsRef<Path>, active_id: &str) -> Result<Self> {
        let base_dir = base_dir.as_ref().to_path_buf();
        let mut map = BTreeMap::new();

        if base_dir.exists() {
            for entry in fs::read_dir(&base_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    let toml_file = path.join("character.toml");
                    if toml_file.exists() {
                        let content = fs::read_to_string(&toml_file)?;
                        match toml::from_str::<CharacterConfig>(&content) {
                            Ok(char_cfg) => {
                                map.insert(char_cfg.id.clone(), char_cfg);
                            }
                            Err(e) => {
                                tracing::warn!("Failed to parse {}: {e}", toml_file.display());
                            }
                        }
                    }
                }
            }
        }

        if map.is_empty() {
            tracing::warn!(
                "No characters found in {}. Creating fallback 'murasame'.",
                base_dir.display()
            );
            let default_char = CharacterConfig::default_for("murasame", "丛雨");
            default_char.save_to_disk(&base_dir)?;
            map.insert(default_char.id.clone(), default_char);
        }

        let verified_active_id = if map.contains_key(active_id) {
            active_id.to_string()
        } else {
            map.keys().next().unwrap().clone()
        };

        Ok(Self {
            active_id: verified_active_id,
            characters: map,
            base_dir,
        })
    }

    pub fn get_active_character(&self) -> Option<&CharacterConfig> {
        self.characters.get(&self.active_id)
    }

    pub fn get_active_character_mut(&mut self) -> Option<&mut CharacterConfig> {
        self.characters.get_mut(&self.active_id)
    }

    pub fn get_character(&self, id: &str) -> Option<&CharacterConfig> {
        self.characters.get(id)
    }

    pub fn get_character_mut(&mut self, id: &str) -> Option<&mut CharacterConfig> {
        self.characters.get_mut(id)
    }

    pub fn switch_active(&mut self, new_id: &str) -> bool {
        if self.characters.contains_key(new_id) {
            self.active_id = new_id.to_string();
            true
        } else {
            false
        }
    }

    pub fn save_character(&self, id: &str) -> Result<()> {
        if let Some(char_cfg) = self.characters.get(id) {
            char_cfg.save_to_disk(&self.base_dir)?;
            Ok(())
        } else {
            bail!("Character '{}' not found", id);
        }
    }

    pub fn save_all(&self) -> Result<()> {
        for char_cfg in self.characters.values() {
            char_cfg.save_to_disk(&self.base_dir)?;
        }
        Ok(())
    }

    pub fn create_character(
        &mut self,
        id: &str,
        name: &str,
        copy_from: Option<&str>,
    ) -> Result<CharacterConfig> {
        let clean_id = id.trim().to_lowercase();
        if clean_id.is_empty() {
            bail!("Character ID cannot be empty");
        }
        if !clean_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            bail!(
                "Character ID must only contain alphanumeric characters, underscores, or hyphens"
            );
        }
        if self.characters.contains_key(&clean_id) {
            bail!("Character ID '{}' already exists", clean_id);
        }

        let new_char = if let Some(source_id) = copy_from {
            if let Some(source_cfg) = self.characters.get(source_id) {
                let cloned = source_cfg.clone_with_new_id(&clean_id, name);
                // Also copy avatar image if available
                let src_avatar = source_cfg.get_avatar_full_path(&self.base_dir);
                let dest_dir = self.base_dir.join(&clean_id);
                fs::create_dir_all(&dest_dir)?;
                if src_avatar.exists() {
                    let dest_avatar = dest_dir.join(&cloned.avatar_path);
                    let _ = fs::copy(&src_avatar, &dest_avatar);
                }
                cloned
            } else {
                CharacterConfig::default_for(&clean_id, name)
            }
        } else {
            CharacterConfig::default_for(&clean_id, name)
        };

        new_char.save_to_disk(&self.base_dir)?;
        self.characters.insert(clean_id.clone(), new_char.clone());
        Ok(new_char)
    }

    pub fn delete_character(&mut self, id: &str) -> Result<()> {
        if self.characters.len() <= 1 {
            bail!("Cannot delete the only remaining character");
        }
        if !self.characters.contains_key(id) {
            bail!("Character '{}' not found", id);
        }

        // If deleting currently active character, switch to another first
        if self.active_id == id {
            let next_id = self.characters.keys().find(|&k| k != id).unwrap().clone();
            self.active_id = next_id;
        }

        self.characters.remove(id);

        let char_dir = self.base_dir.join(id);
        if char_dir.exists() {
            let _ = fs::remove_dir_all(&char_dir);
        }

        Ok(())
    }

    pub fn rename_character(&mut self, id: &str, new_name: &str) -> Result<()> {
        let clean_name = new_name.trim();
        if clean_name.is_empty() {
            bail!("Character name cannot be empty");
        }
        if let Some(char_cfg) = self.characters.get_mut(id) {
            char_cfg.name = clean_name.to_string();
            char_cfg.save_to_disk(&self.base_dir)?;
            Ok(())
        } else {
            bail!("Character '{}' not found", id);
        }
    }

    pub fn list_summaries(&self) -> Vec<CharacterSummary> {
        self.characters
            .values()
            .map(|c| CharacterSummary {
                id: c.id.clone(),
                name: c.name.clone(),
                avatar_path: c.get_avatar_full_path(&self.base_dir),
                is_active: c.id == self.active_id,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_character_manager_lifecycle() -> Result<()> {
        let temp_dir =
            std::env::temp_dir().join(format!("murasame_test_chars_{}", rand::random::<u64>()));
        let _ = fs::remove_dir_all(&temp_dir);

        let mut mgr = CharacterManager::load_all(&temp_dir, "murasame")?;
        assert_eq!(mgr.characters.len(), 1);
        assert_eq!(mgr.active_id, "murasame");

        // Create character
        let eris = mgr.create_character("eris", "艾莉丝", Some("murasame"))?;
        assert_eq!(eris.id, "eris");
        assert_eq!(eris.name, "艾莉丝");
        assert_eq!(mgr.characters.len(), 2);

        // Switch active
        assert!(mgr.switch_active("eris"));
        assert_eq!(mgr.active_id, "eris");
        assert_eq!(mgr.get_active_character().unwrap().name, "艾莉丝");

        // Rename character
        mgr.rename_character("eris", "艾莉丝(新)")?;
        assert_eq!(mgr.get_character("eris").unwrap().name, "艾莉丝(新)");

        // Delete character
        mgr.delete_character("murasame")?;
        assert_eq!(mgr.characters.len(), 1);
        assert_eq!(mgr.active_id, "eris");

        // Cannot delete last character
        assert!(mgr.delete_character("eris").is_err());

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
        Ok(())
    }

    #[test]
    fn test_image_open_and_save() -> Result<()> {
        let avatar_file = Path::new("characters/murasame/avatar.png");
        if avatar_file.exists() {
            let img = image::ImageReader::open(avatar_file)?
                .with_guessed_format()?
                .decode()?;
            let temp_dest = std::env::temp_dir().join("test_saved_avatar.png");
            img.save(&temp_dest)?;
            assert!(temp_dest.exists());
            let _ = fs::remove_file(temp_dest);
        }
        Ok(())
    }
}
