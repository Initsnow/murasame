use rand::seq::SliceRandom;
use std::{collections::HashMap, path::PathBuf};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CueAction {
    pub expressions: Vec<PathBuf>,
    pub motions: Vec<PathBuf>,
    pub motion_groups: Vec<String>,
}

impl CueAction {
    pub fn is_empty(&self) -> bool {
        self.expressions.is_empty() && self.motions.is_empty() && self.motion_groups.is_empty()
    }

    pub fn pick_expression(&self) -> Option<&PathBuf> {
        let mut rng = rand::thread_rng();
        self.expressions.choose(&mut rng)
    }

    pub fn pick_motion(&self) -> Option<&PathBuf> {
        let mut rng = rand::thread_rng();
        self.motions.choose(&mut rng)
    }

    pub fn pick_motion_group(&self) -> Option<&String> {
        let mut rng = rand::thread_rng();
        self.motion_groups.choose(&mut rng)
    }

    pub fn first_expression(&self) -> Option<&PathBuf> {
        self.expressions.first()
    }

    pub fn first_motion(&self) -> Option<&PathBuf> {
        self.motions.first()
    }

    pub fn first_motion_group(&self) -> Option<&String> {
        self.motion_groups.first()
    }
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
        let entry = self.cues.entry(cue.into()).or_default();
        for exp in action.expressions {
            if !entry.expressions.contains(&exp) {
                entry.expressions.push(exp);
            }
        }
        for mot in action.motions {
            if !entry.motions.contains(&mot) {
                entry.motions.push(mot);
            }
        }
        for grp in action.motion_groups {
            if !entry.motion_groups.contains(&grp) {
                entry.motion_groups.push(grp);
            }
        }
    }

    pub fn register_expression(&mut self, cue: impl Into<String>, path: PathBuf) {
        let entry = self.cues.entry(cue.into()).or_default();
        if !entry.expressions.contains(&path) {
            entry.expressions.push(path);
        }
    }

    pub fn register_motion(&mut self, cue: impl Into<String>, path: PathBuf) {
        let entry = self.cues.entry(cue.into()).or_default();
        if !entry.motions.contains(&path) {
            entry.motions.push(path);
        }
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
        self.get(cue).and_then(|a| a.pick_expression())
    }

    pub fn get_motion(&self, cue: &str) -> Option<&PathBuf> {
        self.get(cue).and_then(|a| a.pick_motion())
    }

    pub fn is_expression(&self, cue: &str) -> bool {
        self.get(cue).map_or(false, |a| !a.expressions.is_empty())
    }

    pub fn is_motion(&self, cue: &str) -> bool {
        self.get(cue)
            .map_or(false, |a| !a.motions.is_empty() || !a.motion_groups.is_empty())
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
            .filter(|(_, a)| !a.expressions.is_empty())
            .map(|(k, _)| k.clone())
            .collect();
        keys.sort();
        keys
    }

    pub fn motion_cues(&self) -> Vec<String> {
        let mut keys: Vec<String> = self
            .cues
            .iter()
            .filter(|(_, a)| !a.motions.is_empty() || !a.motion_groups.is_empty())
            .map(|(k, _)| k.clone())
            .collect();
        keys.sort();
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cue_registry_multi_candidate_and_pick() {
        let mut registry = CueRegistry::new();
        let mut action = CueAction::default();
        action.motions.push(PathBuf::from("motion02.motion3.json"));
        action.motions.push(PathBuf::from("motion06.motion3.json"));
        action.expressions.push(PathBuf::from("exp1.exp3"));
        registry.register("tilt_head", action);

        // Registering another motion to same cue merges it without duplicates
        registry.register_motion("tilt_head", PathBuf::from("motion06.motion3.json"));
        registry.register_motion("tilt_head", PathBuf::from("motion03.motion3.json"));

        let retrieved = registry.get("tilt_head").expect("retrieved");
        assert_eq!(retrieved.expressions.len(), 1);
        assert_eq!(retrieved.motions.len(), 3);

        assert_eq!(retrieved.pick_expression().unwrap(), &PathBuf::from("exp1.exp3"));
        let picked_mot = retrieved.pick_motion().unwrap();
        assert!(retrieved.motions.contains(picked_mot));
    }
}
