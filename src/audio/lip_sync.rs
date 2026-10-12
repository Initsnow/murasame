pub struct LipSyncFilter {
    current_value: f32,
    gain: f32,
    noise_threshold: f32,
    attack: f32,
    decay: f32,
}

impl Default for LipSyncFilter {
    fn default() -> Self {
        Self {
            current_value: 0.0,
            gain: 4.5,
            noise_threshold: 0.012,
            attack: 0.65,
            decay: 0.25,
        }
    }
}

impl LipSyncFilter {
    pub fn new(gain: f32) -> Self {
        Self {
            gain,
            ..Self::default()
        }
    }

    pub fn update(&mut self, raw_rms: f32) -> f32 {
        let effective_rms = if raw_rms < self.noise_threshold {
            0.0
        } else {
            (raw_rms - self.noise_threshold) * self.gain
        };

        let target = effective_rms.clamp(0.0, 1.0);

        if target > self.current_value {
            self.current_value += (target - self.current_value) * self.attack;
        } else {
            self.current_value += (target - self.current_value) * self.decay;
        }

        self.current_value.clamp(0.0, 1.0)
    }

    pub fn reset(&mut self) {
        self.current_value = 0.0;
    }
}
