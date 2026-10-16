use std::{collections::HashMap, path::PathBuf};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CueAction {
    pub expression: Option<PathBuf>,
    pub motion: Option<PathBuf>,
    pub motion_group: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CueRegistry {
    cues: HashMap<String, CueAction>,
}

impl CueRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, cue: impl Into<String>, action: CueAction) {
        self.cues.insert(cue.into(), action);
    }

    pub fn register_expression(&mut self, cue: impl Into<String>, path: PathBuf) {
        let entry = self.cues.entry(cue.into()).or_default();
        entry.expression = Some(path);
    }

    pub fn register_motion(&mut self, cue: impl Into<String>, path: PathBuf) {
        let entry = self.cues.entry(cue.into()).or_default();
        entry.motion = Some(path);
    }

    pub fn get(&self, cue: &str) -> Option<&CueAction> {
        let trimmed = cue.trim();
        if let Some(action) = self.cues.get(trimmed) {
            return Some(action);
        }
        for (k, v) in &self.cues {
            if k.eq_ignore_ascii_case(trimmed) {
                return Some(v);
            }
        }
        None
    }

    pub fn get_expression(&self, cue: &str) -> Option<&PathBuf> {
        self.get(cue).and_then(|a| a.expression.as_ref())
    }

    pub fn get_motion(&self, cue: &str) -> Option<&PathBuf> {
        self.get(cue).and_then(|a| a.motion.as_ref())
    }

    pub fn is_expression(&self, cue: &str) -> bool {
        self.get(cue).map_or(false, |a| a.expression.is_some())
    }

    pub fn is_motion(&self, cue: &str) -> bool {
        self.get(cue)
            .map_or(false, |a| a.motion.is_some() || a.motion_group.is_some())
    }

    pub fn is_valid_cue(&self, cue: &str) -> bool {
        let trimmed = cue.trim();
        self.cues.contains_key(trimmed) || self.cues.keys().any(|k| k.eq_ignore_ascii_case(trimmed))
    }

    pub fn all_cues(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.cues.keys().cloned().collect();
        keys.sort();
        keys
    }

    pub fn expression_cues(&self) -> Vec<String> {
        let mut keys: Vec<String> = self
            .cues
            .iter()
            .filter(|(_, a)| a.expression.is_some())
            .map(|(k, _)| k.clone())
            .collect();
        keys.sort();
        keys
    }

    pub fn motion_cues(&self) -> Vec<String> {
        let mut keys: Vec<String> = self
            .cues
            .iter()
            .filter(|(_, a)| a.motion.is_some() || a.motion_group.is_some())
            .map(|(k, _)| k.clone())
            .collect();
        keys.sort();
        keys
    }
}
