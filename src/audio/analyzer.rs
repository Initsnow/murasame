use anyhow::{Result, anyhow};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct F0Stats {
    pub median_f0: f32,
    pub mean_f0: f32,
    pub min_f0: f32,
    pub max_f0: f32,
    pub voiced_ratio: f32,
}

impl F0Stats {
    pub fn note_name(&self) -> String {
        freq_to_note_name(self.median_f0)
    }
}

/// 将频率转换为音符名称，例如 440.0 -> "A4", 261.6 -> "C4"
pub fn freq_to_note_name(freq: f32) -> String {
    if freq <= 10.0 {
        return "N/A".to_string();
    }
    let note_names = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let midi = 12.0 * (freq / 440.0).log2() + 69.0;
    let midi_round = midi.round() as i32;
    if midi_round < 0 || midi_round > 127 {
        return format!("{freq:.1}Hz");
    }
    let note_idx = (midi_round % 12) as usize;
    let octave = (midi_round / 12) - 1;
    let cents = ((midi - midi_round as f32) * 100.0).round() as i32;
    if cents.abs() > 3 {
        format!("{}{}{:+}c", note_names[note_idx], octave, cents)
    } else {
        format!("{}{}", note_names[note_idx], octave)
    }
}

/// 基于归一化自相关算法 (NACF) 检测语音音频中的基频 F0 统计数据
pub fn detect_f0(samples: &[f32], sample_rate: u32) -> Option<F0Stats> {
    if samples.is_empty() || sample_rate == 0 {
        return None;
    }

    let min_freq = 70.0f32;
    let max_freq = 600.0f32;
    let frame_len = (sample_rate as f32 * 0.035).round() as usize; // 35ms 窗口
    let hop_len = (sample_rate as f32 * 0.015).round() as usize; // 15ms 步进

    let min_lag = (sample_rate as f32 / max_freq).floor() as usize;
    let max_lag = (sample_rate as f32 / min_freq).ceil() as usize;

    if samples.len() < frame_len + max_lag {
        return None;
    }

    // 计算全局峰值 RMS 以过滤静音段
    let mut peak_frame_rms = 0.0f32;
    let mut pos = 0;
    while pos + frame_len <= samples.len() {
        let frame = &samples[pos..pos + frame_len];
        let sum_sq: f32 = frame.iter().map(|&x| x * x).sum();
        let rms = (sum_sq / frame_len as f32).sqrt();
        if rms > peak_frame_rms {
            peak_frame_rms = rms;
        }
        pos += hop_len;
    }

    let silence_threshold = (peak_frame_rms * 0.08).max(0.005);

    let mut voiced_f0s = Vec::new();
    let mut total_frames = 0;

    pos = 0;
    while pos + frame_len + max_lag <= samples.len() {
        total_frames += 1;
        let frame = &samples[pos..pos + frame_len];
        let sum_sq: f32 = frame.iter().map(|&x| x * x).sum();
        let rms = (sum_sq / frame_len as f32).sqrt();

        if rms >= silence_threshold {
            // 计算自相关系数
            let mut best_lag = 0usize;
            let mut best_val = -1.0f32;
            let mut correlations = vec![0.0f32; max_lag + 2];

            for lag in min_lag..=max_lag {
                let mut sum_prod = 0.0f32;
                let mut sum_sq_lag = 0.0f32;
                for i in 0..(frame_len - lag) {
                    let s1 = frame[i];
                    let s2 = frame[i + lag];
                    sum_prod += s1 * s2;
                    sum_sq_lag += s2 * s2;
                }
                let norm = (sum_sq * sum_sq_lag).sqrt();
                let r = if norm > 1e-8 { sum_prod / norm } else { 0.0 };
                correlations[lag] = r;

                // 寻找局部峰值
                if lag > min_lag
                    && correlations[lag - 1] > correlations[lag - 2]
                    && correlations[lag - 1] > r
                {
                    let peak_l = lag - 1;
                    let peak_v = correlations[peak_l];
                    if peak_v > best_val && peak_v >= 0.40 {
                        // 避免倍频程错误（检查是否有近一半 lag 处的候选峰值）
                        let half_l = peak_l / 2;
                        let mut half_ok = false;
                        if half_l >= min_lag {
                            for cand in (half_l.saturating_sub(2))..=(half_l + 2).min(max_lag) {
                                if correlations[cand] > 0.82 * peak_v && correlations[cand] > 0.38 {
                                    best_lag = cand;
                                    best_val = correlations[cand];
                                    half_ok = true;
                                    break;
                                }
                            }
                        }
                        if !half_ok {
                            best_lag = peak_l;
                            best_val = peak_v;
                        }
                    }
                }
            }

            if best_lag >= min_lag && best_val >= 0.40 {
                // 抛物线亚采样插值优化峰值精度
                let alpha = correlations[best_lag.saturating_sub(1)];
                let beta = correlations[best_lag];
                let gamma = correlations[(best_lag + 1).min(max_lag)];
                let denom = 2.0 * (alpha - 2.0 * beta + gamma);
                let delta = if denom.abs() > 1e-6 {
                    (alpha - gamma) / denom
                } else {
                    0.0
                };
                let exact_lag = best_lag as f32 + delta;
                let f0 = sample_rate as f32 / exact_lag;
                if f0 >= min_freq && f0 <= max_freq {
                    voiced_f0s.push(f0);
                }
            }
        }
        pos += hop_len;
    }

    if voiced_f0s.len() < 3 {
        return None;
    }

    voiced_f0s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = voiced_f0s[voiced_f0s.len() / 2];
    let sum: f32 = voiced_f0s.iter().sum();
    let mean = sum / voiced_f0s.len() as f32;
    let min = *voiced_f0s.first().unwrap();
    let max = *voiced_f0s.last().unwrap();
    let voiced_ratio = voiced_f0s.len() as f32 / total_frames.max(1) as f32;

    Some(F0Stats {
        median_f0: median,
        mean_f0: mean,
        min_f0: min,
        max_f0: max,
        voiced_ratio,
    })
}

/// 计算非静音段的有效语音 RMS（均方根能量）
pub fn calculate_active_rms(samples: &[f32], sample_rate: u32) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let frame_len = (sample_rate as f32 * 0.03).round() as usize; // 30ms
    let hop_len = frame_len / 2;

    let mut peak_rms = 0.0f32;
    let mut frame_rms_list = Vec::new();

    let mut pos = 0;
    while pos + frame_len <= samples.len() {
        let frame = &samples[pos..pos + frame_len];
        let sum_sq: f32 = frame.iter().map(|&x| x * x).sum();
        let rms = (sum_sq / frame_len as f32).sqrt();
        if rms > peak_rms {
            peak_rms = rms;
        }
        frame_rms_list.push(rms);
        pos += hop_len;
    }

    let thresh = (peak_rms * 0.1).max(0.005);
    let active: Vec<f32> = frame_rms_list
        .into_iter()
        .filter(|&r| r >= thresh)
        .collect();
    if active.is_empty() {
        return peak_rms;
    }
    let sum_active_sq: f32 = active.iter().map(|&r| r * r).sum();
    (sum_active_sq / active.len() as f32).sqrt()
}

/// 根据参考音频 F0 与当前音频 F0 计算推荐的 Beatrice pitch_shift 偏移值 (单位: 半音)
pub fn recommend_pitch_shift(ref_f0: f32, source_f0: f32, current_shift: f64) -> f64 {
    if ref_f0 <= 10.0 || source_f0 <= 10.0 {
        return current_shift;
    }
    let delta_semitones = 12.0 * (ref_f0 / source_f0).log2() as f64;
    current_shift + delta_semitones
}

/// 根据参考音频 RMS 与当前音频 RMS 推荐音量增益倍数
pub fn recommend_volume_multiplier(ref_rms: f32, source_rms: f32, current_volume: f32) -> f32 {
    if ref_rms <= 0.001 || source_rms <= 0.001 {
        return current_volume;
    }
    let ratio = ref_rms / source_rms;
    (current_volume * ratio).clamp(0.2, 2.5)
}

/// 计算语音在共振峰敏感频段 (800Hz ~ 4000Hz) 的能量加权谱质心 (Spectral Centroid)
/// 此区域主要由 F2, F3, F4 决定，能有效反映声道长度 (VTL) 以及音色的幼态/成熟/明暗程度
pub fn calculate_formant_centroid(samples: &[f32], sample_rate: u32) -> Option<f32> {
    if samples.is_empty() || sample_rate < 8000 {
        return None;
    }

    let frame_len = (sample_rate as f32 * 0.035).round() as usize; // 35ms
    let hop_len = (sample_rate as f32 * 0.015).round() as usize; // 15ms
    if samples.len() < frame_len {
        return None;
    }

    // 峰值 RMS 过滤静音
    let mut peak_rms = 0.0f32;
    let mut pos = 0;
    while pos + frame_len <= samples.len() {
        let frame = &samples[pos..pos + frame_len];
        let sum_sq: f32 = frame.iter().map(|&x| x * x).sum();
        let rms = (sum_sq / frame_len as f32).sqrt();
        if rms > peak_rms {
            peak_rms = rms;
        }
        pos += hop_len;
    }
    let silence_thresh = (peak_rms * 0.1).max(0.005);

    // 选取 800Hz 到 4000Hz 的 32 个等距频点
    let min_freq = 800.0f32;
    let max_freq = 4000.0f32;
    let num_bins = 32;
    let step = (max_freq - min_freq) / (num_bins - 1) as f32;
    let mut freqs = Vec::with_capacity(num_bins);
    for i in 0..num_bins {
        freqs.push(min_freq + i as f32 * step);
    }

    // 汉宁窗
    let mut window = Vec::with_capacity(frame_len);
    for n in 0..frame_len {
        let w =
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * n as f32 / (frame_len - 1) as f32).cos());
        window.push(w);
    }

    let mut total_power = vec![0.0f32; num_bins];
    let mut voiced_count = 0;

    pos = 0;
    while pos + frame_len <= samples.len() {
        let frame = &samples[pos..pos + frame_len];
        let sum_sq: f32 = frame.iter().map(|&x| x * x).sum();
        let rms = (sum_sq / frame_len as f32).sqrt();

        if rms >= silence_thresh {
            voiced_count += 1;
            for (bin_idx, &freq) in freqs.iter().enumerate() {
                let omega = 2.0 * std::f32::consts::PI * freq / sample_rate as f32;
                let mut real = 0.0f32;
                let mut imag = 0.0f32;
                for n in 0..frame_len {
                    let val = frame[n] * window[n];
                    let angle = omega * n as f32;
                    real += val * angle.cos();
                    imag -= val * angle.sin();
                }
                let p = real * real + imag * imag;
                total_power[bin_idx] += p;
            }
        }
        pos += hop_len;
    }

    if voiced_count < 3 {
        return None;
    }

    let sum_p: f32 = total_power.iter().sum();
    if sum_p <= 1e-8 {
        return None;
    }

    let weighted_sum: f32 = freqs
        .iter()
        .zip(total_power.iter())
        .map(|(&f, &p)| f * p)
        .sum();
    Some(weighted_sum / sum_p)
}

/// 根据参考音频与源音频的共振峰质心推荐 Beatrice formant_shift (半音)
pub fn recommend_formant_shift(ref_centroid: f32, source_centroid: f32, current_shift: f64) -> f64 {
    if ref_centroid <= 100.0 || source_centroid <= 100.0 {
        return current_shift;
    }
    let delta_semitones = 12.0 * (ref_centroid / source_centroid).log2() as f64;
    // 共振峰调整做适当阻尼 (0.85)，防止跨语种/文本元音分布不均导致过度偏移，并限制在 Beatrice 模型支持的 [-2.0, 2.0]
    let damped_delta = delta_semitones * 0.85;
    (current_shift + damped_delta).clamp(-2.0, 2.0)
}

/// 读取音频文件 (支持 WAV 与 WebM/Opus)，转换为单声道 f32 PCM
pub fn load_audio_file(path: impl AsRef<Path>) -> Result<(u32, Vec<f32>)> {
    let p = path.as_ref();
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ext == "wav" {
        crate::audio::player::read_wav_to_f32_pcm(p)
    } else if ext == "webm" {
        let bytes = std::fs::read(p)?;
        crate::audio::tts::decode_webm_opus_to_f32_pcm(&bytes)
    } else {
        // 尝试按 wav 读取，若失败尝试 WebM/Opus
        if let Ok(res) = crate::audio::player::read_wav_to_f32_pcm(p) {
            Ok(res)
        } else {
            let bytes = std::fs::read(p)?;
            crate::audio::tts::decode_webm_opus_to_f32_pcm(&bytes)
        }
    }
}

/// 保存单声道 f32 PCM 为 16-bit WAV 文件
pub fn save_wav_file(path: impl AsRef<Path>, samples: &[f32], sample_rate: u32) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    if let Some(parent) = path.as_ref().parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut writer = hound::WavWriter::create(path.as_ref(), spec)
        .map_err(|e| anyhow!("Failed to create WAV file: {e}"))?;
    for &sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let s = (clamped * 32767.0).round() as i16;
        writer.write_sample(s)?;
    }
    writer.finalize()?;
    Ok(())
}
