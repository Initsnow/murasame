use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use super::serde_helpers::*;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CueMapping {
    #[serde(
        default,
        alias = "expressions",
        serialize_with = "serialize_string_or_vec",
        deserialize_with = "deserialize_string_or_vec"
    )]
    pub expression: Vec<String>,
    #[serde(
        default,
        alias = "motions",
        serialize_with = "serialize_string_or_vec",
        deserialize_with = "deserialize_string_or_vec"
    )]
    pub motion: Vec<String>,
    #[serde(
        default,
        alias = "motion_groups",
        serialize_with = "serialize_string_or_vec",
        deserialize_with = "deserialize_string_or_vec"
    )]
    pub motion_group: Vec<String>,
}

impl CueMapping {
    pub fn from_single(
        exp: impl Into<String>,
        mot: impl Into<String>,
        grp: impl Into<String>,
    ) -> Self {
        let e = exp.into().trim().to_string();
        let m = mot.into().trim().to_string();
        let g = grp.into().trim().to_string();
        Self {
            expression: if e.is_empty() { vec![] } else { vec![e] },
            motion: if m.is_empty() { vec![] } else { vec![m] },
            motion_group: if g.is_empty() { vec![] } else { vec![g] },
        }
    }

    pub fn from_strings(exp: &str, mot: &str, grp: &str) -> Self {
        let split_items = |s: &str| -> Vec<String> {
            s.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        };
        Self {
            expression: split_items(exp),
            motion: split_items(mot),
            motion_group: split_items(grp),
        }
    }

    pub fn first_expression(&self) -> &str {
        self.expression.first().map(|s| s.as_str()).unwrap_or("")
    }

    pub fn first_motion(&self) -> &str {
        self.motion.first().map(|s| s.as_str()).unwrap_or("")
    }

    pub fn first_motion_group(&self) -> &str {
        self.motion_group.first().map(|s| s.as_str()).unwrap_or("")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CueConfig {
    #[serde(default, serialize_with = "serialize_sorted_map")]
    pub mappings: HashMap<String, CueMapping>,
    #[serde(
        default,
        serialize_with = "serialize_hashmap_string_or_vec",
        deserialize_with = "deserialize_hashmap_string_or_vec"
    )]
    pub expressions: HashMap<String, Vec<String>>,
    #[serde(
        default,
        serialize_with = "serialize_hashmap_string_or_vec",
        deserialize_with = "deserialize_hashmap_string_or_vec"
    )]
    pub motions: HashMap<String, Vec<String>>,
}

impl CueConfig {
    pub fn normalized_mappings(&self) -> HashMap<String, CueMapping> {
        let mut map = self.mappings.clone();
        for (k, v) in &self.expressions {
            let entry = map.entry(k.clone()).or_default();
            for exp in v {
                if !entry.expression.contains(exp) {
                    entry.expression.push(exp.clone());
                }
            }
        }
        for (k, v) in &self.motions {
            let entry = map.entry(k.clone()).or_default();
            for mot in v {
                if !entry.motion.contains(mot) {
                    entry.motion.push(mot.clone());
                }
            }
        }
        map
    }
}
