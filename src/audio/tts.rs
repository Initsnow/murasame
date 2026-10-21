use std::io::Cursor;
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Result, anyhow};
use edge_tts_rust::{EdgeTtsClient, EdgeTtsClientBuilder, SpeakOptions, SynthesisEvent};
use futures_util::StreamExt;
use rubato::{FastFixedIn, PolynomialDegree, Resampler};
use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::formats::FormatOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

static CODEC_REGISTRY: LazyLock<CodecRegistry> = LazyLock::new(|| {
    let mut registry = CodecRegistry::new();
    symphonia::default::register_enabled_codecs(&mut registry);
    registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
    registry
});

pub struct TtsEngine {
    client: EdgeTtsClient,
}

impl TtsEngine {
    pub fn new() -> Result<Self> {
        let client = EdgeTtsClientBuilder::default()
            .connect_timeout(Duration::from_secs(10))
            .receive_timeout(Duration::from_secs(30))
            .ws_pool_size(1)
            .ws_warmup(true)
            .request_chunk_reuse(true)
            .build()
            .map_err(|e| anyhow!("Failed to initialize Edge TTS client: {e}"))?;

        Ok(Self { client })
    }

    pub async fn synthesize_to_pcm(
        &self,
        text: &str,
        voice: &str,
        rate: &str,
        pitch: &str,
        volume: &str,
    ) -> Result<Vec<f32>> {
        let cleaned = clean_text_for_tts(text);
        let trimmed_text = cleaned.trim();
        if trimmed_text.is_empty() {
            return Ok(Vec::new());
        }

        // 1. 配置 Edge TTS 参数 (使用 WebM Opus 24kHz)
        let options = SpeakOptions {
            voice: voice.to_string(),
            rate: rate.to_string(),
            pitch: pitch.to_string(),
            volume: volume.to_string(),
            output_format: "webm-24khz-16bit-mono-opus".to_string(),
            ..SpeakOptions::default()
        };

        // 2. 网络请求重试与指数退避，最大程度避免 429 失败导致音频丢失
        let max_retries = 3;
        let mut last_error = None;
        let mut webm_bytes = Vec::new();

        for attempt in 1..=max_retries {
            webm_bytes.clear();
            let stream_result = self.client.stream(trimmed_text, options.clone()).await;

            match stream_result {
                Ok(mut stream) => {
                    let mut stream_err = None;
                    while let Some(event_res) = stream.next().await {
                        match event_res {
                            Ok(SynthesisEvent::Audio(chunk)) => {
                                webm_bytes.extend_from_slice(&chunk);
                            }
                            Ok(_) => {}
                            Err(e) => {
                                stream_err = Some(anyhow!("TTS synthesis stream error: {e}"));
                                break;
                            }
                        }
                    }

                    if let Some(err) = stream_err {
                        tracing::warn!(
                            "Edge TTS synthesis attempt {attempt}/{max_retries} stream error: {err}"
                        );
                        last_error = Some(err);
                    } else if webm_bytes.is_empty() {
                        let err = anyhow!("Edge TTS returned empty audio data");
                        tracing::warn!(
                            "Edge TTS synthesis attempt {attempt}/{max_retries}: received empty audio"
                        );
                        last_error = Some(err);
                    } else {
                        last_error = None;
                        break;
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Edge TTS synthesis attempt {attempt}/{max_retries} connection failed: {e}"
                    );
                    last_error = Some(anyhow!("Edge TTS stream connection error: {e}"));
                }
            }

            if attempt < max_retries {
                let jitter = rand::random::<u64>() % 100;
                let backoff_ms = (attempt as u64 * 250) + jitter;
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            }
        }

        if let Some(err) = last_error {
            return Err(err);
        }

        // 3. 使用 symphonia + symphonia-adapter-libopus 解码 WebM Opus (在 blocking 线程执行以隔离 C 栈开销)
        let (sample_rate, raw_pcm) = tokio::task::spawn_blocking(move || {
            decode_webm_opus_to_f32_pcm(&webm_bytes)
        })
        .await
        .map_err(|e| anyhow!("TTS decoding task failed: {e}"))??;

        // 4. 确保输出为 24000Hz 单声道 (Beatrice 变声模型与播放管线要求 24kHz)
        let pcm_24k = if sample_rate != 24000 {
            tokio::task::spawn_blocking(move || resample_to_24k(&raw_pcm, sample_rate))
                .await
                .map_err(|e| anyhow!("TTS resampling task failed: {e}"))??
        } else {
            raw_pcm
        };

        Ok(pcm_24k)
    }
}

/// 判断字符是否为常见的 Emoji、表情符号、变体选择器等
pub fn is_emoji(c: char) -> bool {
    let u = c as u32;
    matches!(u,
        // Emoticons, Misc Symbols & Pictographs, Transport, Supplemental, etc.
        0x1F600..=0x1F64F |
        0x1F300..=0x1F5FF |
        0x1F680..=0x1F6FF |
        0x1F700..=0x1F77F |
        0x1F780..=0x1F7FF |
        0x1F800..=0x1F8FF |
        0x1F900..=0x1F9FF |
        0x1FA00..=0x1FAFF |
        0x2600..=0x26FF |
        0x2700..=0x27BF |
        0xFE00..=0xFE0F |
        0x1F1E6..=0x1F1FF |
        0x200D |
        0x20E3
    )
}

/// 将包含 Markdown 格式、Emoji、Live2D 标签等的输入文本转换为适合 TTS 语音朗读的纯文本
pub fn clean_text_for_tts(input: &str) -> String {
    let mut text = input.to_string();

    // 1. 移除多行代码块 ```...```
    while let Some(start) = text.find("```") {
        if let Some(end) = text[start + 3..].find("```") {
            text.replace_range(start..start + 3 + end + 3, " ");
        } else {
            text.truncate(start);
            break;
        }
    }

    // 2. 移除行内代码反引号 `code` -> code
    text = text.replace('`', "");

    // 3. 处理图片 ![alt](url) -> 移除
    while let Some(start) = text.find("![") {
        if let Some(mid) = text[start..].find("](") {
            if let Some(end) = text[start + mid..].find(')') {
                text.replace_range(start..start + mid + end + 1, " ");
                continue;
            }
        }
        break;
    }

    // 4. 处理链接 [text](url) -> text
    while let Some(start) = text.find('[') {
        if let Some(mid) = text[start..].find("](") {
            if let Some(end) = text[start + mid..].find(')') {
                let link_text = text[start + 1..start + mid].to_string();
                text.replace_range(start..start + mid + end + 1, &link_text);
                continue;
            }
        }
        break;
    }

    // 5. 移除可能的 Live2D 标签或残留的 [tag]（非 markdown 链接的独立方括号）
    while let Some(start) = text.find('[') {
        if let Some(end) = text[start..].find(']') {
            let inside = &text[start + 1..start + end];
            if inside.len() <= 64 && !inside.contains('\n') {
                text.replace_range(start..start + end + 1, " ");
                continue;
            }
        }
        break;
    }

    // 6. 移除 Markdown 粗体/斜体/删除线标记: **, __, ~~, *, _
    text = text.replace("**", "");
    text = text.replace("__", "");
    text = text.replace("~~", "");
    text = text.replace('*', "");
    text = text.replace('_', "");


    let mut result = String::with_capacity(text.len());
    let mut in_html_tag = false;

    // 逐字符过滤 Emoji、HTML 标签、Markdown 标题符号 '#'
    for ch in text.chars() {
        if ch == '<' {
            in_html_tag = true;
            continue;
        } else if ch == '>' {
            in_html_tag = false;
            continue;
        } else if in_html_tag {
            continue;
        }

        if is_emoji(ch) {
            continue;
        }

        if ch == '#' {
            continue;
        }

        result.push(ch);
    }

    // 8. 清理多余空格与不可见空白字符
    let mut cleaned = String::with_capacity(result.len());
    let mut last_was_space = false;
    for ch in result.chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                cleaned.push(' ');
                last_was_space = true;
            }
        } else {
            cleaned.push(ch);
            last_was_space = false;
        }
    }

    cleaned.trim().to_string()
}

/// 使用 symphonia 及 symphonia-adapter-libopus 将 WebM (Opus) 音频数据解码为单声道 f32 PCM
pub fn decode_webm_opus_to_f32_pcm(webm_bytes: &[u8]) -> Result<(u32, Vec<f32>)> {
    if webm_bytes.is_empty() {
        return Ok((24000, Vec::new()));
    }

    let cursor = Box::new(Cursor::new(webm_bytes.to_vec()));
    let mss = MediaSourceStream::new(cursor, Default::default());

    let mut hint = Hint::new();
    hint.with_extension("webm");

    let probe = symphonia::default::get_probe();
    let mut format = probe
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| anyhow!("Failed to probe WebM audio format: {e}"))?;

    let (track_id, audio_params) = format
        .tracks()
        .iter()
        .find_map(|t| match &t.codec_params {
            Some(CodecParameters::Audio(params)) => Some((t.id, params.clone())),
            _ => None,
        })
        .ok_or_else(|| anyhow!("No audio track found in WebM data"))?;

    let mut decoder = CODEC_REGISTRY
        .make_audio_decoder(&audio_params, &AudioDecoderOptions::default())
        .map_err(|e| anyhow!("Failed to instantiate Opus decoder: {e}"))?;

    let mut sample_rate = audio_params.sample_rate.unwrap_or(24000);
    let mut pcm = Vec::new();
    let mut temp_interleaved = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(symphonia::core::errors::Error::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(_) => {
                break;
            }
        };

        if packet.track_id != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                sample_rate = decoded.spec().rate();
                let channels = decoded.spec().channels().count();

                temp_interleaved.clear();
                decoded.copy_to_vec_interleaved(&mut temp_interleaved);

                if channels == 1 {
                    pcm.extend_from_slice(&temp_interleaved);
                } else if channels == 2 {
                    for chunk in temp_interleaved.chunks_exact(2) {
                        pcm.push((chunk[0] + chunk[1]) * 0.5);
                    }
                } else if channels > 2 {
                    for chunk in temp_interleaved.chunks_exact(channels) {
                        let sum: f32 = chunk.iter().sum();
                        pcm.push(sum / channels as f32);
                    }
                }
            }
            Err(symphonia::core::errors::Error::DecodeError(e)) => {
                tracing::warn!("Opus decode warning on packet: {e}");
                continue;
            }
            Err(symphonia::core::errors::Error::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(e) => {
                tracing::warn!("Opus decode packet error: {e}");
                continue;
            }
        }
    }

    Ok((sample_rate, pcm))
}

/// 使用 Rubato 将 PCM 转换为 24000Hz 单声道
pub fn resample_to_24k(samples: &[f32], from_rate: u32) -> Result<Vec<f32>> {
    if from_rate == 24000 || samples.is_empty() {
        return Ok(samples.to_vec());
    }

    let f_ratio = 24000.0 / from_rate as f64;
    let chunk_size = 1024;
    let mut resampler = FastFixedIn::<f32>::new(
        f_ratio,
        1.5,
        PolynomialDegree::Cubic,
        chunk_size,
        1,
    )
    .map_err(|e| anyhow!("Failed to initialize rubato resampler: {e}"))?;

    let mut output = Vec::with_capacity((samples.len() as f64 * f_ratio) as usize + chunk_size);
    let mut offset = 0;
    while offset < samples.len() {
        let end = (offset + chunk_size).min(samples.len());
        let chunk = &samples[offset..end];
        let mut in_buffer = vec![0.0f32; chunk_size];
        in_buffer[..chunk.len()].copy_from_slice(chunk);
        let waves = resampler
            .process(&[&in_buffer], None)
            .map_err(|e| anyhow!("Rubato resample error: {e}"))?;

        if !waves.is_empty() && !waves[0].is_empty() {
            let actual_len = if end == samples.len() {
                ((chunk.len() as f64 * f_ratio).round() as usize).min(waves[0].len())
            } else {
                waves[0].len()
            };
            output.extend_from_slice(&waves[0][..actual_len]);
        }
        offset += chunk_size;
    }

    Ok(output)
}
