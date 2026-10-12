use std::collections::HashMap;
use crate::config::InteractionConfig;
use super::loader::HitAreaEntry;

#[derive(Debug, Clone)]
pub struct HitAreaHandler {
    /// Hit area name (e.g. "face", "hair") -> motion group (e.g. "Tapface", "Taphair")
    area_to_motion_group: HashMap<String, String>,
}

impl HitAreaHandler {
    pub fn new(declared: &[HitAreaEntry], config: &InteractionConfig) -> Self {
        let mut area_to_motion_group = HashMap::new();

        // 1. Defaults from model3.json HitAreas declaration
        for area in declared {
            if let Some(motion) = &area.motion {
                area_to_motion_group.insert(area.name.clone(), motion.clone());
            } else {
                area_to_motion_group.insert(area.name.clone(), format!("Tap{}", area.name));
            }
        }

        // 2. User overrides from config.toml
        for (area_name, motion_group) in &config.hit_areas {
            area_to_motion_group.insert(area_name.clone(), motion_group.clone());
        }

        Self {
            area_to_motion_group,
        }
    }

    pub fn get_motion_group_for_area(&self, area_name: &str) -> Option<&String> {
        self.area_to_motion_group.get(area_name)
    }
}
