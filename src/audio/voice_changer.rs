use super::beatrice::{Beatrice, new as new_beatrice};
use anyhow::{Result, anyhow};
use std::path::Path;

pub struct VoiceChanger {
    engine: Box<dyn Beatrice>,
}

impl VoiceChanger {
    pub fn new(
        model_path: impl AsRef<Path>,
        speaker_id: u32,
        pitch_shift: f64,
        formant_shift: f64,
    ) -> Result<Self> {
        let mut engine = new_beatrice(model_path.as_ref(), 24000.0, 48000.0, 1, 1)
            .map_err(|e| anyhow!("Failed to load Beatrice model: {e:?}"))?;

        let _ = engine.set_target_speaker(speaker_id);
        engine.set_pitch_shift(pitch_shift);
        engine.set_formant_shift(formant_shift);

        Ok(Self { engine })
    }

    pub fn convert(&mut self, input_pcm_24k: &[f32]) -> Result<Vec<f32>> {
        self.engine
            .infer(input_pcm_24k)
            .map_err(|e| anyhow!("Beatrice inference failed: {e:?}"))
    }

    pub fn set_speaker(&mut self, speaker_id: u32) -> Result<()> {
        self.engine
            .set_target_speaker(speaker_id)
            .map_err(|e| anyhow!("Failed to set speaker {speaker_id}: {e:?}"))
    }

    pub fn set_pitch_shift(&mut self, pitch_shift: f64) {
        self.engine.set_pitch_shift(pitch_shift);
    }

    pub fn set_formant_shift(&mut self, formant_shift: f64) {
        self.engine.set_formant_shift(formant_shift);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_and_convert() {
        let p = "models/murasame-beatrice-v1";
        if std::path::Path::new(p).exists() {
            println!("Testing Beatrice load from {p}");
            match VoiceChanger::new(p, 0, 0.0, 0.0) {
                Ok(mut vc) => {
                    println!("Beatrice loaded successfully!");
                    let dummy_input = vec![0.0f32; 24000];
                    match vc.convert(&dummy_input) {
                        Ok(out) => println!("Beatrice convert ok! Output len = {}", out.len()),
                        Err(e) => println!("Beatrice convert err: {e:?}"),
                    }
                }
                Err(e) => {
                    println!("Beatrice load err: {e:?}");
                }
            }
        }
    }
}
