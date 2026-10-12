use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};
use anyhow::{Context, Result};
use serde::Deserialize;

use crate::config::{CueConfig, InteractionConfig};
use super::cue::CueRegistry;

#[derive(Debug, Deserialize)]
pub struct Model3Json {
    #[serde(rename = "Version")]
    pub version: u32,
    #[serde(rename = "FileReferences")]
    pub file_references: FileReferences,
    #[serde(rename = "HitAreas", default)]
    pub hit_areas: Vec<HitAreaEntry>,
}

#[derive(Debug, Deserialize)]
pub struct FileReferences {
    #[serde(rename = "Moc")]
    pub moc: String,
    #[serde(rename = "Textures")]
    pub textures: Vec<String>,
    #[serde(rename = "Physics")]
    pub physics: Option<String>,
    #[serde(rename = "Motions", default)]
    pub motions: HashMap<String, Vec<MotionEntry>>,
    #[serde(rename = "Expressions", default)]
    pub expressions: Vec<ExpressionEntry>,
}

#[derive(Debug, Deserialize)]
pub struct MotionEntry {
    #[serde(rename = "File")]
    pub file: String,
    #[serde(rename = "Sound")]
    pub sound: Option<String>,
    #[serde(rename = "Text")]
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExpressionEntry {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "File")]
    pub file: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HitAreaEntry {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Motion")]
    pub motion: Option<String>,
}

#[derive(Debug)]
pub struct ModelCatalog {
    pub model_dir: PathBuf,
    pub model_file: PathBuf,
    pub hit_areas: Vec<HitAreaEntry>,
    pub motion_groups: HashMap<String, Vec<PathBuf>>,
    pub available_motions: HashMap<String, PathBuf>,
    pub available_expressions: HashMap<String, PathBuf>,
    pub cues: CueRegistry,
}

impl ModelCatalog {
    pub fn load(
        model_path: impl AsRef<Path>,
        cue_config: &CueConfig,
        _interaction_config: &InteractionConfig,
    ) -> Result<Self> {
        let model_file = model_path.as_ref().to_path_buf();
        let model_dir = model_file
            .parent()
            .context("Model path has no parent directory")?
            .to_path_buf();

        let content = fs::read_to_string(&model_file)
            .with_context(|| format!("Failed to read model file: {}", model_file.display()))?;
        let model_json: Model3Json = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse model JSON: {}", model_file.display()))?;

        let mut motion_groups: HashMap<String, Vec<PathBuf>> = HashMap::new();
        let mut available_motions: HashMap<String, PathBuf> = HashMap::new();

        for (group_name, entries) in &model_json.file_references.motions {
            let mut list = Vec::new();
            for entry in entries {
                let full_path = model_dir.join(&entry.file);
                list.push(full_path.clone());

                let file_name = Path::new(&entry.file)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&entry.file)
                    .to_string();
                available_motions.insert(file_name, full_path.clone());
                available_motions.insert(entry.file.clone(), full_path);
            }
            motion_groups.insert(group_name.clone(), list);
        }

        let mut available_expressions: HashMap<String, PathBuf> = HashMap::new();
        for entry in &model_json.file_references.expressions {
            let full_path = model_dir.join(&entry.file);
            let file_name = Path::new(&entry.file)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&entry.file)
                .to_string();
            available_expressions.insert(entry.name.clone(), full_path.clone());
            available_expressions.insert(file_name, full_path.clone());
            available_expressions.insert(entry.file.clone(), full_path);
        }

        // Also check if neutral.exp3.json exists in root of model_dir
        let neutral_candidate = model_dir.join("neutral.exp3.json");
        if neutral_candidate.exists() {
            available_expressions.insert("neutral.exp3".to_string(), neutral_candidate.clone());
            available_expressions.insert("neutral.exp3.json".to_string(), neutral_candidate);
        }

        let mut cues = CueRegistry::new();

        // Register configured expression cues
        for (cue_name, target) in &cue_config.expressions {
            if let Some(path) = available_expressions.get(target) {
                cues.register_expression(cue_name, path.clone());
            } else {
                let candidate = model_dir.join(target);
                if candidate.exists() {
                    cues.register_expression(cue_name, candidate);
                } else {
                    tracing::warn!("Configured expression cue '{}' -> '{}' not found in model", cue_name, target);
                }
            }
        }

        // Register configured motion cues
        for (cue_name, target) in &cue_config.motions {
            if let Some(path) = available_motions.get(target) {
                cues.register_motion(cue_name, path.clone());
            } else {
                let candidate = model_dir.join(target);
                if candidate.exists() {
                    cues.register_motion(cue_name, candidate);
                } else {
                    tracing::warn!("Configured motion cue '{}' -> '{}' not found in model", cue_name, target);
                }
            }
        }

        Ok(Self {
            model_dir,
            model_file,
            hit_areas: model_json.hit_areas,
            motion_groups,
            available_motions,
            available_expressions,
            cues,
        })
    }
}
