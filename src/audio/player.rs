use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Duration,
};
use anyhow::{Result, anyhow};
use rodio::{OutputStreamHandle, Sink, Source};

pub struct AudioPlayer {
    stream_handle: OutputStreamHandle,
    current_sink: Option<Sink>,
    rms_meter: Arc<AtomicU32>,
    is_playing: Arc<AtomicBool>,
}

impl AudioPlayer {
    pub fn new(stream_handle: OutputStreamHandle) -> Self {
        Self {
            stream_handle,
            current_sink: None,
            rms_meter: Arc::new(AtomicU32::new(0)),
            is_playing: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn play_wav_file(&mut self, path: impl AsRef<std::path::Path>, volume: f32) -> Result<()> {
        let (rate, samples) = read_wav_to_f32_pcm(path)?;
        self.play_samples(samples, rate, volume)
    }

    pub fn play_samples(&mut self, samples: Vec<f32>, sample_rate: u32, volume: f32) -> Result<()> {
        self.stop();

        let sink = Sink::try_new(&self.stream_handle)
            .map_err(|e| anyhow!("Failed to create audio sink: {e}"))?;
        sink.set_volume(volume);

        let source = RmsTrackingSource::new(
            samples,
            sample_rate,
            Arc::clone(&self.rms_meter),
            Arc::clone(&self.is_playing),
        );

        sink.append(source);
        self.current_sink = Some(sink);
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(sink) = self.current_sink.take() {
            sink.stop();
        }
        self.rms_meter.store(0, Ordering::Relaxed);
        self.is_playing.store(false, Ordering::Relaxed);
    }

    pub fn current_rms(&self) -> f32 {
        f32::from_bits(self.rms_meter.load(Ordering::Relaxed))
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Relaxed)
    }
}

struct RmsTrackingSource {
    samples: Vec<f32>,
    index: usize,
    sample_rate: u32,
    rms_meter: Arc<AtomicU32>,
    is_playing: Arc<AtomicBool>,
    window_size: usize,
    current_window_count: usize,
    sum_squares: f32,
}

impl RmsTrackingSource {
    fn new(
        samples: Vec<f32>,
        sample_rate: u32,
        rms_meter: Arc<AtomicU32>,
        is_playing: Arc<AtomicBool>,
    ) -> Self {
        is_playing.store(!samples.is_empty(), Ordering::Relaxed);
        Self {
            samples,
            index: 0,
            sample_rate,
            rms_meter,
            is_playing,
            window_size: 1024,
            current_window_count: 0,
            sum_squares: 0.0,
        }
    }
}

impl Iterator for RmsTrackingSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.samples.len() {
            let sample = self.samples[self.index];
            self.index += 1;

            self.sum_squares += sample * sample;
            self.current_window_count += 1;

            if self.current_window_count >= self.window_size {
                let mean_square = self.sum_squares / self.current_window_count as f32;
                let rms = mean_square.sqrt();
                self.rms_meter.store(rms.to_bits(), Ordering::Relaxed);
                self.sum_squares = 0.0;
                self.current_window_count = 0;
            }

            Some(sample)
        } else {
            self.rms_meter.store(0, Ordering::Relaxed);
            self.is_playing.store(false, Ordering::Relaxed);
            None
        }
    }
}

impl Source for RmsTrackingSource {
    fn current_frame_len(&self) -> Option<usize> {
        Some(self.samples.len() - self.index)
    }

    fn channels(&self) -> u16 {
        1
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        let secs = self.samples.len() as f64 / self.sample_rate as f64;
        Some(Duration::from_secs_f64(secs))
    }
}

pub fn read_wav_to_f32_pcm(wav_path: impl AsRef<std::path::Path>) -> Result<(u32, Vec<f32>)> {
    let mut reader = hound::WavReader::open(wav_path.as_ref())
        .map_err(|e| anyhow!("Failed to open wav file {}: {e}", wav_path.as_ref().display()))?;
    let spec = reader.spec();
    let sample_rate = spec.sample_rate;
    let mut pcm = Vec::new();

    match spec.sample_format {
        hound::SampleFormat::Int => {
            let max_val = (1i64 << (spec.bits_per_sample - 1)) as f32;
            for s in reader.samples::<i32>() {
                let sample = s.map_err(|e| anyhow!("WAV sample read error: {e}"))?;
                pcm.push(sample as f32 / max_val);
            }
        }
        hound::SampleFormat::Float => {
            for s in reader.samples::<f32>() {
                let sample = s.map_err(|e| anyhow!("WAV sample read error: {e}"))?;
                pcm.push(sample);
            }
        }
    }

    if spec.channels == 2 {
        let mono: Vec<f32> = pcm.chunks_exact(2).map(|c| (c[0] + c[1]) * 0.5).collect();
        return Ok((sample_rate, mono));
    }

    Ok((sample_rate, pcm))
}
