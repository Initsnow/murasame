use std::{collections::HashMap, path::PathBuf};

#[derive(Debug, Clone, Default)]
pub struct CueRegistry {
    /// Cue name (e.g. "smile") -> full path to expression file
    expressions: HashMap<String, PathBuf>,
    /// Cue name (e.g. "greet") -> full path to motion file
    motions: HashMap<String, PathBuf>,
}

impl CueRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_expression(&mut self, cue: impl Into<String>, path: PathBuf) {
        self.expressions.insert(cue.into(), path);
    }

    pub fn register_motion(&mut self, cue: impl Into<String>, path: PathBuf) {
        self.motions.insert(cue.into(), path);
    }

    pub fn get_expression(&self, cue: &str) -> Option<&PathBuf> {
        self.expressions.get(cue)
    }

    pub fn get_motion(&self, cue: &str) -> Option<&PathBuf> {
        self.motions.get(cue)
    }

    pub fn is_expression(&self, cue: &str) -> bool {
        self.expressions.contains_key(cue)
    }

    pub fn is_motion(&self, cue: &str) -> bool {
        self.motions.contains_key(cue)
    }

    pub fn is_valid_cue(&self, cue: &str) -> bool {
        self.is_expression(cue) || self.is_motion(cue)
    }

    pub fn expression_cues(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.expressions.keys().cloned().collect();
        keys.sort();
        keys
    }

    pub fn motion_cues(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.motions.keys().cloned().collect();
        keys.sort();
        keys
    }
}
