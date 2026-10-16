use anyhow::{Context, Result};
use serde::Deserialize;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use super::cue::CueRegistry;
use crate::config::{CueConfig, InteractionConfig};

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
    pub model_bounds: Option<crate::renderer::ModelBounds>,
}

impl ModelCatalog {
    pub fn get_random_motion_from_group(&self, group: &str) -> Option<PathBuf> {
        if let Some(list) = self.motion_groups.get(group) {
            if !list.is_empty() {
                use rand::seq::SliceRandom;
                let mut rng = rand::thread_rng();
                return list.choose(&mut rng).cloned();
            }
        }
        None
    }

    pub fn get_random_motion_from_any(&self) -> Option<PathBuf> {
        use rand::seq::SliceRandom;
        let mut rng = rand::thread_rng();
        let paths: Vec<&PathBuf> = self.available_motions.values().collect();
        paths.choose(&mut rng).map(|p| (*p).clone())
    }

    pub fn calculate_fitted_window_size(&self, target_height: f32) -> (f32, f32) {
        if let Some(bounds) = self.model_bounds {
            bounds.calculate_fitted_dimensions(target_height)
        } else {
            ((target_height * 0.5).round(), target_height)
        }
    }
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
        let normalized_cues = cue_config.normalized_mappings();

        for (cue_name, mapping) in &normalized_cues {
            let mut action = crate::model::cue::CueAction::default();

            if !mapping.expression.is_empty() {
                if let Some(path) = available_expressions.get(&mapping.expression) {
                    action.expression = Some(path.clone());
                } else {
                    let candidate = model_dir.join(&mapping.expression);
                    if candidate.exists() {
                        action.expression = Some(candidate);
                    } else {
                        tracing::warn!(
                            "Expression cue '{cue_name}' -> '{}' not found in model",
                            mapping.expression
                        );
                    }
                }
            }

            if !mapping.motion.is_empty() {
                if let Some(path) = available_motions.get(&mapping.motion) {
                    action.motion = Some(path.clone());
                } else {
                    let candidate = model_dir.join(&mapping.motion);
                    if candidate.exists() {
                        action.motion = Some(candidate);
                    } else {
                        tracing::warn!(
                            "Motion cue '{cue_name}' -> '{}' not found in model",
                            mapping.motion
                        );
                    }
                }
            }

            if !mapping.motion_group.is_empty() {
                if motion_groups.contains_key(&mapping.motion_group) {
                    action.motion_group = Some(mapping.motion_group.clone());
                } else {
                    tracing::warn!(
                        "Motion group cue '{cue_name}' -> '{}' not found in model",
                        mapping.motion_group
                    );
                }
            }

            if action.expression.is_some()
                || action.motion.is_some()
                || action.motion_group.is_some()
            {
                cues.register(cue_name, action);
            }
        }

        // Extract real model mesh bounds for dynamic fitted window sizing
        let model_bounds = mocari::assets::load_model_runtime(&model_file)
            .ok()
            .and_then(|loaded| {
                crate::renderer::ModelBounds::from_drawables(loaded.runtime().meshes())
            });

        Ok(Self {
            model_dir,
            model_file,
            hit_areas: model_json.hit_areas,
            motion_groups,
            available_motions,
            available_expressions,
            cues,
            model_bounds,
        })
    }
}
