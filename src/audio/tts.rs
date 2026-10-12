use std::io::Cursor;
use anyhow::{Result, anyhow};
use edge_tts_rust::{EdgeTtsClient, SpeakOptions, SynthesisEvent};
use futures_util::StreamExt;
use minimp3::{Decoder, Error, Frame};

pub struct TtsEngine {
    client: EdgeTtsClient,
}

impl TtsEngine {
    pub fn new() -> Result<Self> {
        let client = EdgeTtsClient::new()
            .map_err(|e| anyhow!("Failed to initialize Edge TTS client: {e}"))?;
        Ok(Self { client })
    }

    pub async fn synthesize_to_pcm(&self, text: &str, voice: &str) -> Result<Vec<f32>> {
        let options = SpeakOptions {
            voice: voice.to_string(),
            output_format: "audio-24khz-48kbitrate-mono-mp3".to_string(),
            ..SpeakOptions::default()
        };

        let mut stream = self
            .client
            .stream(text, options)
            .await
            .map_err(|e| anyhow!("Failed to create TTS stream: {e}"))?;

        let mut mp3_bytes = Vec::new();
        while let Some(event_res) = stream.next().await {
            match event_res {
                Ok(SynthesisEvent::Audio(chunk)) => {
                    mp3_bytes.extend_from_slice(&chunk);
                }
                Ok(_) => {}
                Err(e) => return Err(anyhow!("Error in TTS synthesis stream: {e}")),
            }
        }

        if mp3_bytes.is_empty() {
            return Err(anyhow!("TTS returned no audio data"));
        }

        let (_sample_rate, pcm) = decode_mp3_to_f32_pcm(&mp3_bytes)?;
        Ok(pcm)
    }
}

pub fn decode_mp3_to_f32_pcm(mp3_bytes: &[u8]) -> Result<(u32, Vec<f32>)> {
    let mut decoder = Decoder::new(Cursor::new(mp3_bytes));
    let mut pcm = Vec::new();
    let mut sample_rate = 24000;

    loop {
        match decoder.next_frame() {
            Ok(Frame {
                data,
                sample_rate: rate,
                channels,
                ..
            }) => {
                sample_rate = rate as u32;
                if channels == 1 {
                    for s in data {
                        pcm.push(s as f32 / 32768.0);
                    }
                } else if channels == 2 {
                    for chunk in data.chunks_exact(2) {
                        let mono = (chunk[0] as f32 + chunk[1] as f32) / (2.0 * 32768.0);
                        pcm.push(mono);
                    }
                }
            }
            Err(Error::Eof) => break,
            Err(e) => return Err(anyhow!("MP3 decode error: {e}")),
        }
    }

    Ok((sample_rate, pcm))
}
