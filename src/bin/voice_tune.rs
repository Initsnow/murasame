use anyhow::{Result, anyhow};
use rodio::OutputStream;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use murasame::{
    audio::{
        AudioPlayer, TtsEngine, VoiceChanger,
        analyzer::{
            F0Stats, calculate_active_rms, calculate_formant_centroid, detect_f0,
            freq_to_note_name, load_audio_file, recommend_formant_shift, recommend_pitch_shift,
            recommend_volume_multiplier, save_wav_file,
        },
    },
    character::{CharacterConfig, CharacterManager},
};

#[derive(Clone, PartialEq, Eq, Hash)]
struct TtsCacheKey {
    text: String,
    voice: String,
    rate: String,
    pitch: String,
    volume: String,
}

struct TtsCache {
    memory_key: Option<TtsCacheKey>,
    memory_pcm: Option<Vec<f32>>,
    cache_dir: PathBuf,
}

impl TtsCache {
    fn new(cache_dir: impl AsRef<Path>) -> Self {
        let cache_dir = cache_dir.as_ref().to_path_buf();
        let _ = std::fs::create_dir_all(&cache_dir);
        Self {
            memory_key: None,
            memory_pcm: None,
            cache_dir,
        }
    }

    fn key_hash(key: &TtsCacheKey) -> u64 {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish()
    }

    async fn get_or_synthesize(&mut self, tts: &TtsEngine, key: TtsCacheKey) -> Result<Vec<f32>> {
        // 1. 内存缓存检查
        if let Some(ref mem_key) = self.memory_key {
            if mem_key == &key {
                if let Some(ref pcm) = self.memory_pcm {
                    println!(
                        "\x1b[32m⚡ [TTS 缓存命中(内存)]\x1b[0m 基础语音参数未变，复用 24kHz PCM (0ms)"
                    );
                    return Ok(pcm.clone());
                }
            }
        }

        // 2. 磁盘缓存检查
        let hash = Self::key_hash(&key);
        let disk_path = self.cache_dir.join(format!("tts_{hash:016x}.wav"));
        if disk_path.exists() {
            if let Ok((_sr, pcm)) = load_audio_file(&disk_path) {
                println!("\x1b[36m💾 [TTS 缓存命中(磁盘)]\x1b[0m 从本地文件加载已合成音频 (1ms)");
                self.memory_key = Some(key);
                self.memory_pcm = Some(pcm.clone());
                return Ok(pcm);
            }
        }

        // 3. 云端重新合成
        println!("\x1b[33m🌐 [TTS 云端合成中...]\x1b[0m 正在向 Edge-TTS 请求音频...");
        let start = Instant::now();
        let pcm = tts
            .synthesize_to_pcm(&key.text, &key.voice, &key.rate, &key.pitch, &key.volume)
            .await?;
        let elapsed = start.elapsed().as_millis();
        println!(
            "\x1b[32m✓ [TTS 合成完毕]\x1b[0m 耗时: {elapsed}ms (采样点数: {}, 采样率: 24kHz)",
            pcm.len()
        );

        // 写入磁盘与内存缓存
        let _ = save_wav_file(&disk_path, &pcm, 24000);
        self.memory_key = Some(key);
        self.memory_pcm = Some(pcm.clone());

        Ok(pcm)
    }
}

struct Session {
    char_id: String,
    char_cfg: CharacterConfig,
    base_dir: PathBuf,
    ref_audio_path: Option<PathBuf>,
    ref_pcm: Option<Vec<f32>>,
    ref_sample_rate: u32,
    ref_f0: Option<F0Stats>,
    ref_formant_centroid: Option<f32>,
    ref_rms: f32,
    test_text: String,

    // 当前合成缓存
    last_synth_pcm: Option<Vec<f32>>,
    last_synth_f0: Option<F0Stats>,
    last_synth_formant_centroid: Option<f32>,
    last_synth_rms: f32,

    // 引擎
    tts_engine: TtsEngine,
    tts_cache: TtsCache,
    voice_changer: Option<VoiceChanger>,
    audio_player: AudioPlayer,
    _stream: rodio::OutputStream,
}

impl Session {
    fn new(char_id: &str, ref_path_opt: Option<PathBuf>) -> Result<Self> {
        let base_dir = murasame::paths::app_root_dir().join("characters");
        let mgr = CharacterManager::load_all(&base_dir, char_id)?;
        let char_cfg = mgr
            .get_character(char_id)
            .cloned()
            .ok_or_else(|| anyhow!("未找到角色配置: '{char_id}'"))?;

        let (_stream, stream_handle) =
            OutputStream::try_default().map_err(|e| anyhow!("无法初始化系统音频输出设备: {e}"))?;
        let audio_player = AudioPlayer::new(stream_handle);
        let tts_engine = TtsEngine::new()?;
        let tts_cache = TtsCache::new(".cache/voice_tune");

        // 默认测试台词
        let default_text = "大家好啊，今天来点大家想看的东西啊".to_string();

        let mut session = Self {
            char_id: char_id.to_string(),
            char_cfg,
            base_dir,
            ref_audio_path: None,
            ref_pcm: None,
            ref_sample_rate: 48000,
            ref_f0: None,
            ref_formant_centroid: None,
            ref_rms: 0.0,
            test_text: default_text,
            last_synth_pcm: None,
            last_synth_f0: None,
            last_synth_formant_centroid: None,
            last_synth_rms: 0.0,
            tts_engine,
            tts_cache,
            voice_changer: None,
            audio_player,
            _stream,
        };

        // 载入或自动查找参考音频
        if let Some(ref_path) = ref_path_opt {
            session.set_reference_audio(ref_path)?;
        } else {
            session.auto_find_reference_audio();
        }

        // 初始化 Beatrice 变声器
        session.init_or_update_beatrice(false)?;

        Ok(session)
    }

    fn auto_find_reference_audio(&mut self) {
        let candidate_dirs = [
            PathBuf::from("Murasame_live2d/sounds"),
            self.base_dir.join(&self.char_id).join("sounds"),
            self.base_dir.join(&self.char_id),
            PathBuf::from("assets/sounds"),
        ];

        for dir in candidate_dirs {
            if dir.exists() {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        let ext = path
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("")
                            .to_lowercase();
                        if ext == "wav" || ext == "mp3" {
                            println!("🔍 自动发现参考音频: {}", path.display());
                            let _ = self.set_reference_audio(path);
                            return;
                        }
                    }
                }
            }
        }
    }

    fn set_reference_audio(&mut self, path: PathBuf) -> Result<()> {
        if !path.exists() {
            return Err(anyhow!("音频文件不存在: {}", path.display()));
        }
        let (sample_rate, pcm) = load_audio_file(&path)?;
        let f0 = detect_f0(&pcm, sample_rate);
        let formant_centroid = calculate_formant_centroid(&pcm, sample_rate);
        let rms = calculate_active_rms(&pcm, sample_rate);

        self.ref_audio_path = Some(path.clone());
        self.ref_sample_rate = sample_rate;
        self.ref_pcm = Some(pcm);
        self.ref_f0 = f0;
        self.ref_formant_centroid = formant_centroid;
        self.ref_rms = rms;

        println!("\x1b[32m✓ 成功载入参考音频:\x1b[0m {}", path.display());
        let f0_str = if let Some(ref f) = self.ref_f0 {
            format!("基频 F0 = {:.1} Hz ({})", f.median_f0, f.note_name())
        } else {
            "基频 F0 = 未检出".to_string()
        };
        let fc_str = if let Some(fc) = self.ref_formant_centroid {
            format!("共振峰中心 = {:.0} Hz", fc)
        } else {
            "共振峰中心 = 未检出".to_string()
        };
        println!(
            "   参考声学特征: {}, {}, 有效响度 RMS = {:.4}",
            f0_str, fc_str, self.ref_rms
        );

        Ok(())
    }

    fn init_or_update_beatrice(&mut self, force_reload: bool) -> Result<()> {
        if !self.char_cfg.audio.beatrice_enabled {
            println!("\x1b[33m提示: 当前角色配置中 Beatrice 未启用\x1b[0m");
            return Ok(());
        }

        let model_path =
            murasame::paths::resolve_resource_path(&self.char_cfg.audio.beatrice_model_path);
        if !model_path.exists() {
            return Err(anyhow!("Beatrice 模型路径不存在: {}", model_path.display()));
        }

        if force_reload || self.voice_changer.is_none() {
            println!("⏳ 正在加载 Beatrice 模型: {}...", model_path.display());
            let start = Instant::now();
            let vc = VoiceChanger::new(
                &model_path,
                self.char_cfg.audio.speaker_id,
                self.char_cfg.audio.pitch_shift,
                self.char_cfg.audio.formant_shift,
            )?;
            println!(
                "\x1b[32m✓ Beatrice 模型就绪\x1b[0m (耗时 {}ms)",
                start.elapsed().as_millis()
            );
            self.voice_changer = Some(vc);
        } else if let Some(ref mut vc) = self.voice_changer {
            let _ = vc.set_speaker(self.char_cfg.audio.speaker_id);
            vc.set_pitch_shift(self.char_cfg.audio.pitch_shift);
            vc.set_formant_shift(self.char_cfg.audio.formant_shift);
        }

        Ok(())
    }

    async fn synthesize_current(&mut self) -> Result<Vec<f32>> {
        let key = TtsCacheKey {
            text: self.test_text.clone(),
            voice: self.char_cfg.audio.tts_voice.clone(),
            rate: self.char_cfg.audio.tts_rate.clone(),
            pitch: self.char_cfg.audio.tts_pitch.clone(),
            volume: self.char_cfg.audio.tts_volume.clone(),
        };

        // 获取 TTS 基础音频 (24kHz)
        let tts_pcm = self
            .tts_cache
            .get_or_synthesize(&self.tts_engine, key)
            .await?;

        // 下游 Beatrice 变声处理 (24kHz -> 48kHz)
        let final_pcm = if self.char_cfg.audio.beatrice_enabled {
            self.init_or_update_beatrice(false)?;
            if let Some(ref mut vc) = self.voice_changer {
                let start = Instant::now();
                let converted = vc.convert(&tts_pcm)?;
                let elapsed = start.elapsed().as_millis();
                println!("⚡ [Beatrice 推理完成] 耗时: {elapsed}ms (输出采样率: 48kHz)");
                converted
            } else {
                tts_pcm
            }
        } else {
            tts_pcm
        };

        let sample_rate = if self.char_cfg.audio.beatrice_enabled {
            48000
        } else {
            24000
        };
        let f0 = detect_f0(&final_pcm, sample_rate);
        let formant_centroid = calculate_formant_centroid(&final_pcm, sample_rate);
        let rms = calculate_active_rms(&final_pcm, sample_rate);

        self.last_synth_pcm = Some(final_pcm.clone());
        self.last_synth_f0 = f0;
        self.last_synth_formant_centroid = formant_centroid;
        self.last_synth_rms = rms;

        Ok(final_pcm)
    }

    fn play_synthesized(&mut self) -> Result<()> {
        if let Some(ref pcm) = self.last_synth_pcm {
            let sr = if self.char_cfg.audio.beatrice_enabled {
                48000
            } else {
                24000
            };
            let vol = self.char_cfg.audio.volume;
            println!("▶ 播放合成语音 (音量: {:.0}%)...", vol * 100.0);
            self.audio_player.play_samples(pcm.clone(), sr, vol)?;
            Ok(())
        } else {
            Err(anyhow!("尚未生成合成语音，请先合成"))
        }
    }

    fn play_reference(&mut self) -> Result<()> {
        if let Some(ref pcm) = self.ref_pcm {
            println!("▶ 播放参考原声音频...");
            self.audio_player
                .play_samples(pcm.clone(), self.ref_sample_rate, 1.0)?;
            Ok(())
        } else {
            Err(anyhow!("未加载参考音频，请使用 'ref <文件路径>' 设置"))
        }
    }

    async fn play_ab_comparison(&mut self) -> Result<()> {
        if self.ref_pcm.is_none() {
            return Err(anyhow!("未加载参考音频，无法进行 A/B 对比"));
        }

        // 确保有最新合成
        if self.last_synth_pcm.is_none() {
            let _ = self.synthesize_current().await?;
        }

        println!("\x1b[1;36m===== [A/B 对比播放] =====\x1b[0m");
        println!("\x1b[33m[A] 播放参考原声 (Ref)\x1b[0m");
        self.play_reference()?;

        // 等待参考播放结束
        while self.audio_player.is_playing() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        // 停顿 350ms
        tokio::time::sleep(Duration::from_millis(350)).await;

        println!("\x1b[32m[B] 播放合成语音 (Synth)\x1b[0m");
        self.play_synthesized()?;

        while self.audio_player.is_playing() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        println!("\x1b[1;36m===== [A/B 对比结束] =====\x1b[0m");

        Ok(())
    }

    async fn auto_tune(
        &mut self,
        tune_pitch: bool,
        tune_formant: bool,
        tune_vol: bool,
    ) -> Result<()> {
        if self.ref_pcm.is_none() {
            return Err(anyhow!("未加载参考音频，无法进行自动声学分析"));
        }

        // 先确保有当前的合成并分析
        if self.last_synth_pcm.is_none() {
            let _ = self.synthesize_current().await?;
        }

        let ref_f0_val = self.ref_f0.as_ref().map(|f| f.median_f0).unwrap_or(0.0);
        let synth_f0_val = self
            .last_synth_f0
            .as_ref()
            .map(|f| f.median_f0)
            .unwrap_or(0.0);

        println!("\n\x1b[1;35m──────── 自动声学匹配分析 ────────\x1b[0m");

        // 1. 音高基频 F0 自动匹配
        if tune_pitch {
            if ref_f0_val > 10.0 && synth_f0_val > 10.0 {
                let current_shift = self.char_cfg.audio.pitch_shift;
                let recommended_shift =
                    recommend_pitch_shift(ref_f0_val, synth_f0_val, current_shift);
                let diff = recommended_shift - current_shift;

                let ref_note = freq_to_note_name(ref_f0_val);
                let synth_note = freq_to_note_name(synth_f0_val);

                println!(
                    "  • 参考音频 F0: \x1b[33m{:.1} Hz ({})\x1b[0m",
                    ref_f0_val, ref_note
                );
                println!(
                    "  • 当前合成 F0: \x1b[36m{:.1} Hz ({})\x1b[0m",
                    synth_f0_val, synth_note
                );
                println!("  • 当前 Pitch Shift: {:+.2} 半音", current_shift);
                println!(
                    "  • 推荐 Pitch Shift: \x1b[1;32m{:+.2}\x1b[0m 半音 (调整量: {:+.2})",
                    recommended_shift, diff
                );

                self.char_cfg.audio.pitch_shift = recommended_shift;
                println!("  \x1b[32m✔ 已自动应用推荐音高偏移！\x1b[0m");
            } else {
                println!("  • 未能从参考或合成音频检测到有效基频 F0，跳过音高调整");
            }
        }

        // 2. 共振峰质心 Formant 自动匹配 (Vocal Tract Length Normalization)
        if tune_formant {
            if let (Some(ref_fc), Some(synth_fc)) =
                (self.ref_formant_centroid, self.last_synth_formant_centroid)
            {
                let current_formant = self.char_cfg.audio.formant_shift;
                let recommended_formant =
                    recommend_formant_shift(ref_fc, synth_fc, current_formant);
                let diff = recommended_formant - current_formant;

                println!(
                    "  • 参考共振峰质心 (F2~F4): \x1b[33m{:.0} Hz\x1b[0m",
                    ref_fc
                );
                println!(
                    "  • 当前合成共振峰质心:     \x1b[36m{:.0} Hz\x1b[0m",
                    synth_fc
                );
                println!("  • 当前 Formant Shift: {:+.2} 半音", current_formant);
                println!(
                    "  • 推荐 Formant Shift: \x1b[1;32m{:+.2}\x1b[0m 半音 (调整量: {:+.2})",
                    recommended_formant, diff
                );

                self.char_cfg.audio.formant_shift = recommended_formant;
                println!("  \x1b[32m✔ 已自动应用推荐共振峰偏移！\x1b[0m");
            } else {
                println!("  • 未能检测到足够的共振峰质心数据，跳过共振峰调整");
            }
        }

        // 3. 有效响度 RMS 自动匹配
        if tune_vol && self.ref_rms > 0.001 && self.last_synth_rms > 0.001 {
            let current_vol = self.char_cfg.audio.volume;
            let rec_vol =
                recommend_volume_multiplier(self.ref_rms, self.last_synth_rms, current_vol);
            println!("  • 参考有效 RMS: {:.4}", self.ref_rms);
            println!("  • 当前合成 RMS: {:.4}", self.last_synth_rms);
            println!(
                "  • 推荐 Volume 增益: \x1b[1;32m{:.2}\x1b[0m (当前: {:.2})",
                rec_vol, current_vol
            );
            self.char_cfg.audio.volume = rec_vol;
            println!("  \x1b[32m✔ 已自动应用推荐输出音量！\x1b[0m");
        }
        println!("\x1b[1;35m──────────────────────────────────\x1b[0m\n");

        // 重新使用当前 Beatrice 推理并自动播放试听
        println!("🔄 正在使用新参数重算并试听...");
        let _ = self.synthesize_current().await?;
        self.play_synthesized()?;

        Ok(())
    }

    async fn run_grid_sweep(&mut self) -> Result<()> {
        let key = TtsCacheKey {
            text: self.test_text.clone(),
            voice: self.char_cfg.audio.tts_voice.clone(),
            rate: self.char_cfg.audio.tts_rate.clone(),
            pitch: self.char_cfg.audio.tts_pitch.clone(),
            volume: self.char_cfg.audio.tts_volume.clone(),
        };

        println!("\n\x1b[1;36m===== 🚀 开始 3x3 参数网格扫描生成 =====\x1b[0m");
        println!("正在获取基础 TTS 音频 (自动使用缓存)...");
        let tts_pcm = self
            .tts_cache
            .get_or_synthesize(&self.tts_engine, key)
            .await?;

        let out_dir = PathBuf::from("tuned_output/sweep");
        std::fs::create_dir_all(&out_dir)?;

        let cur_p = self.char_cfg.audio.pitch_shift;
        let cur_f = self.char_cfg.audio.formant_shift;

        // 生成 3 档音高 x 3 档共振峰
        let pitch_offsets = [cur_p - 1.5, cur_p, cur_p + 1.5];
        let formant_offsets = [cur_f - 0.6, cur_f, cur_f + 0.6];

        self.init_or_update_beatrice(false)?;
        let vc = self
            .voice_changer
            .as_mut()
            .ok_or_else(|| anyhow!("Beatrice 未初始化"))?;

        let sweep_result = (|| -> Result<()> {
            let mut manifest = String::from("Murasame Voice Tuning - Grid Sweep Results\n");
            manifest.push_str("====================================================\n");
            manifest.push_str(&format!("测试台词: {}\n\n", self.test_text));
            manifest.push_str("序号 | 文件名 | Pitch Shift | Formant Shift | 输出 F0 | 响度 RMS\n");
            manifest.push_str("----------------------------------------------------\n");

            let mut count = 0;
            let start_all = Instant::now();

            for &p in &pitch_offsets {
                for &f in &formant_offsets {
                    count += 1;
                    vc.set_pitch_shift(p);
                    vc.set_formant_shift(f);
                    let synth = vc.convert(&tts_pcm)?;

                    let f0_stat = detect_f0(&synth, 48000);
                    let rms = calculate_active_rms(&synth, 48000);

                    let filename = format!("{:02}_p{:+.1}_f{:+.1}.wav", count, p, f);
                    let file_path = out_dir.join(&filename);
                    save_wav_file(&file_path, &synth, 48000)?;

                    let f0_str = if let Some(f0) = f0_stat {
                        format!("{:.1}Hz ({})", f0.median_f0, f0.note_name())
                    } else {
                        "N/A".to_string()
                    };

                    let line = format!(
                        "{:02}   | {:<18} | {:<11.1} | {:<13.1} | {:<12} | {:.4}\n",
                        count, filename, p, f, f0_str, rms
                    );
                    manifest.push_str(&line);
                    print!("  [{:02}/09] 生成: {} -> F0: {}\n", count, filename, f0_str);
                }
            }

            let manifest_path = out_dir.join("index.txt");
            std::fs::write(&manifest_path, &manifest)?;
            println!(
                "\x1b[32m✓ 扫描生成完成!\x1b[0m 耗时: {}ms",
                start_all.elapsed().as_millis()
            );
            Ok(())
        })();

        // 恢复原有参数
        vc.set_pitch_shift(cur_p);
        vc.set_formant_shift(cur_f);

        println!("📂 输出目录: {}", out_dir.display());
        sweep_result?;
        println!(
            "📄 结果清单已保存至: {}\n",
            out_dir.join("index.txt").display()
        );

        Ok(())
    }

    fn print_status(&self) {
        println!(
            "\n\x1b[1;34m╔══════════════════════════════════════════════════════════════╗\x1b[0m"
        );
        println!(
            "\x1b[1;34m║                 Murasame 音色微调调优工具                     ║\x1b[0m"
        );
        println!(
            "\x1b[1;34m╚══════════════════════════════════════════════════════════════╝\x1b[0m"
        );
        println!(
            " 当前角色: \x1b[1m{} ({})\x1b[0m",
            self.char_cfg.name, self.char_id
        );
        println!(
            " 参考音频: \x1b[33m{}\x1b[0m",
            self.ref_audio_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "未指定 (输入 'ref <路径>' 加载)".to_string())
        );
        if let Some(ref f) = self.ref_f0 {
            let fc_str = if let Some(fc) = self.ref_formant_centroid {
                format!(" | 共振峰中心: {:.0} Hz", fc)
            } else {
                "".to_string()
            };
            println!(
                "  ↳ 参考声学特征: \x1b[33m{:.1} Hz ({})\x1b[0m{} | 响度 RMS: {:.4}",
                f.median_f0,
                f.note_name(),
                fc_str,
                self.ref_rms
            );
        }
        println!(" 测试台词: \"\x1b[37m{}\x1b[0m\"", self.test_text);
        println!("────────────────────────────────────────────────────────────────");
        println!("\x1b[1m[Edge-TTS 上游参数]\x1b[0m");
        println!(
            "  发音人 (voice): \x1b[36m{}\x1b[0m | 语速 (rate): {} | 音调 (pitch): {} | 音量: {}",
            self.char_cfg.audio.tts_voice,
            self.char_cfg.audio.tts_rate,
            self.char_cfg.audio.tts_pitch,
            self.char_cfg.audio.tts_volume
        );
        println!("\x1b[1m[Beatrice 下游变声参数]\x1b[0m");
        println!(
            "  启用状态: {} | Speaker ID: {} | 输出音量 (volume): {:.2}",
            if self.char_cfg.audio.beatrice_enabled {
                "\x1b[32m已启用\x1b[0m"
            } else {
                "\x1b[31m未启用\x1b[0m"
            },
            self.char_cfg.audio.speaker_id,
            self.char_cfg.audio.volume
        );
        println!(
            "  变调偏移 (pitch_shift):   \x1b[1;32m{:+.2}\x1b[0m 半音",
            self.char_cfg.audio.pitch_shift
        );
        println!(
            "  共振峰偏移 (formant_shift): \x1b[1;32m{:+.2}\x1b[0m 半音 (正值声音更幼/更亮，负值更沉/更厚)",
            self.char_cfg.audio.formant_shift
        );

        if let Some(ref f) = self.last_synth_f0 {
            println!("────────────────────────────────────────────────────────────────");
            let fc_str = if let Some(fc) = self.last_synth_formant_centroid {
                format!(" | 共振峰中心: \x1b[36m{:.0} Hz\x1b[0m", fc)
            } else {
                "".to_string()
            };
            println!(
                " 最近合成声学指标: F0 = \x1b[32m{:.1} Hz ({})\x1b[0m{} | RMS = {:.4}",
                f.median_f0,
                f.note_name(),
                fc_str,
                self.last_synth_rms
            );
            if let Some(ref rf) = self.ref_f0 {
                let semi_diff = 12.0 * (f.median_f0 / rf.median_f0).log2();
                println!(
                    " 音高基频偏差: {:+.2} 半音 ({:+.1} Hz)",
                    semi_diff,
                    f.median_f0 - rf.median_f0
                );
            }
            if let (Some(ref_fc), Some(synth_fc)) =
                (self.ref_formant_centroid, self.last_synth_formant_centroid)
            {
                let f_diff = 12.0 * (synth_fc / ref_fc).log2();
                println!(
                    " 共振峰偏差:   {:+.2} 半音 ({:+.0} Hz)",
                    f_diff,
                    synth_fc - ref_fc
                );
            }
        }
        println!("────────────────────────────────────────────────────────────────");
    }

    fn print_help(&self) {
        println!("\n\x1b[1;33m快捷指令说明:\x1b[0m");
        println!("  \x1b[1;32m1\x1b[0m                     播放参考原声音频");
        println!(
            "  \x1b[1;32m2\x1b[0m / \x1b[1;32mplay\x1b[0m               合成并试听当前参数 (有缓存时毫秒级)"
        );
        println!(
            "  \x1b[1;32m3\x1b[0m / \x1b[1;32mab\x1b[0m                 A/B 连播对比 (原声 ➔ 停顿 350ms ➔ 合成音)"
        );
        println!(
            "  \x1b[1;32mauto [模式]\x1b[0m           \x1b[1;35m[自动声学对齐]\x1b[0m 自动估算并应用推荐参数"
        );
        println!(
            "                         (模式: auto 默认全对齐 | auto p 音高 | auto f 共振峰 | auto vol 音量)"
        );
        println!(
            "  \x1b[1;32msweep\x1b[0m                 \x1b[1;36m[网格扫描]\x1b[0m 批量生成 3x3 组变调与共振峰切片供盲听"
        );
        println!();
        println!(
            "  \x1b[36mp\x1b[0m / \x1b[36mpitch <数值>\x1b[0m        设置 Beatrice 音高偏移 (如: p 1.5 或 p -0.8)"
        );
        println!(
            "  \x1b[36mf\x1b[0m / \x1b[36mformant <数值>\x1b[0m      设置 Beatrice 共振峰偏移 (如: f 0.8 或 f -0.3)"
        );
        println!("  \x1b[36mvol <数值>\x1b[0m            设置输出音量增益 (如: vol 1.4)");
        println!("  \x1b[36mspk <ID>\x1b[0m              切换 Beatrice Speaker ID (如: spk 0)");
        println!(
            "  \x1b[36mv\x1b[0m / \x1b[36mvoice <发音人>\x1b[0m      切换 Edge-TTS 发音人 (如: v zh-CN-XiaoyiNeural)"
        );
        println!("  \x1b[36mrate <百分比>\x1b[0m         设置 Edge-TTS 语速 (如: rate +15%)");
        println!("  \x1b[36mtts_pitch <Hz/百分比>\x1b[0m 设置 Edge-TTS 音高 (如: tts_pitch +10Hz)");
        println!("  \x1b[36mtext <台词文本>\x1b[0m       更换测试台词 (更换台词后将重新拉取 TTS)");
        println!("  \x1b[36mref <音频路径>\x1b[0m        载入新的参考音频 (WAV/MP3)");
        println!("  \x1b[36mvoices\x1b[0m                查看常用 Edge-TTS 推荐发音人");
        println!("  \x1b[36mstatus\x1b[0m                重新打印当前参数与声学指标面板");
        println!("  \x1b[1;32msave\x1b[0m                  保存当前最优参数到 character.toml");
        println!("  \x1b[31mq\x1b[0m / \x1b[31mquit\x1b[0m                 退出调优工具\n");
    }

    fn save_to_config(&self) -> Result<()> {
        self.char_cfg.save_to_disk(&self.base_dir)?;
        println!(
            "\x1b[32m✔ 成功将配置保存至: {}/{}/character.toml\x1b[0m",
            self.base_dir.display(),
            self.char_id
        );
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // 处理命令行参数
    let args: Vec<String> = std::env::args().collect();
    let mut char_id = "murasame".to_string();
    let mut ref_path: Option<PathBuf> = None;
    let mut auto_mode = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--character" | "-c" => {
                if i + 1 < args.len() {
                    char_id = args[i + 1].clone();
                    i += 1;
                }
            }
            "--ref" | "-r" => {
                if i + 1 < args.len() {
                    ref_path = Some(PathBuf::from(&args[i + 1]));
                    i += 1;
                }
            }
            "--auto" => {
                auto_mode = true;
            }
            "--help" | "-h" => {
                println!("Murasame Voice Tuning CLI");
                println!("用法: voice-tune [选项]");
                println!("选项:");
                println!("  -c, --character <ID>     指定角色 ID (默认: murasame)");
                println!("  -r, --ref <路径>          指定参考原声音频文件 (WAV/MP3)");
                println!("      --auto               一键自动完成音高、共振峰与音量声学匹配并保存");
                println!("  -h, --help               显示帮助信息");
                return Ok(());
            }
            _ => {}
        }
        i += 1;
    }

    let mut session = Session::new(&char_id, ref_path)?;

    if auto_mode {
        println!("🚀 正在执行一键自动化参数匹配 (音高 + 共振峰 + 音量)...");
        session.auto_tune(true, true, true).await?;
        session.save_to_config()?;
        println!("已自动保存，退出。");
        return Ok(());
    }

    session.print_status();
    session.print_help();

    // 交互式循环
    let stdin = io::stdin();
    let mut input = String::new();

    loop {
        print!("\x1b[1;36m[{}@tune]>\x1b[0m ", session.char_id);
        io::stdout().flush()?;
        input.clear();
        if stdin.read_line(&mut input)? == 0 {
            break;
        }

        let line = input.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        let cmd = parts[0].to_lowercase();

        match cmd.as_str() {
            "q" | "quit" | "exit" => {
                println!("再见！");
                break;
            }
            "h" | "help" | "?" => {
                session.print_help();
            }
            "status" => {
                session.print_status();
            }
            "1" => {
                if let Err(e) = session.play_reference() {
                    println!("\x1b[31m错误: {e}\x1b[0m");
                }
            }
            "2" | "play" => match session.synthesize_current().await {
                Ok(_) => {
                    let _ = session.play_synthesized();
                }
                Err(e) => println!("\x1b[31m合成失败: {e}\x1b[0m"),
            },
            "3" | "ab" => {
                if let Err(e) = session.play_ab_comparison().await {
                    println!("\x1b[31m对比播放失败: {e}\x1b[0m");
                }
            }
            "auto" => {
                let sub = if parts.len() > 1 { parts[1] } else { "all" };
                match sub {
                    "p" | "pitch" => {
                        if let Err(e) = session.auto_tune(true, false, false).await {
                            println!("\x1b[31m自动音高对齐失败: {e}\x1b[0m");
                        }
                    }
                    "f" | "formant" => {
                        if let Err(e) = session.auto_tune(false, true, false).await {
                            println!("\x1b[31m自动共振峰对齐失败: {e}\x1b[0m");
                        }
                    }
                    "vol" | "volume" => {
                        if let Err(e) = session.auto_tune(false, false, true).await {
                            println!("\x1b[31m自动音量对齐失败: {e}\x1b[0m");
                        }
                    }
                    _ => {
                        if let Err(e) = session.auto_tune(true, true, true).await {
                            println!("\x1b[31m自动全声学对齐失败: {e}\x1b[0m");
                        }
                    }
                }
            }
            "sweep" => {
                if let Err(e) = session.run_grid_sweep().await {
                    println!("\x1b[31m网格扫描失败: {e}\x1b[0m");
                }
            }
            "p" | "pitch" => {
                if parts.len() > 1 {
                    if let Ok(val) = parts[1].parse::<f64>() {
                        session.char_cfg.audio.pitch_shift = val;
                        println!("✔ Pitch Shift 已设为: {:+.2}", val);
                        if let Err(e) = session.init_or_update_beatrice(false) {
                            println!("\x1b[31mBeatrice 更新失败: {e}\x1b[0m");
                        } else {
                            match session.synthesize_current().await {
                                Ok(_) => {
                                    let _ = session.play_synthesized();
                                }
                                Err(e) => println!("\x1b[31m合成失败: {e}\x1b[0m"),
                            }
                        }
                    } else {
                        println!("格式错误，示例: p 1.5 或 pitch -0.5");
                    }
                } else {
                    println!(
                        "当前 Pitch Shift: {:+.2}",
                        session.char_cfg.audio.pitch_shift
                    );
                }
            }
            "f" | "formant" => {
                if parts.len() > 1 {
                    if let Ok(val) = parts[1].parse::<f64>() {
                        session.char_cfg.audio.formant_shift = val;
                        println!("✔ Formant Shift 已设为: {:+.2}", val);
                        if let Err(e) = session.init_or_update_beatrice(false) {
                            println!("\x1b[31mBeatrice 更新失败: {e}\x1b[0m");
                        } else {
                            match session.synthesize_current().await {
                                Ok(_) => {
                                    let _ = session.play_synthesized();
                                }
                                Err(e) => println!("\x1b[31m合成失败: {e}\x1b[0m"),
                            }
                        }
                    } else {
                        println!("格式错误，示例: f 0.8 或 formant -0.3");
                    }
                } else {
                    println!(
                        "当前 Formant Shift: {:+.2}",
                        session.char_cfg.audio.formant_shift
                    );
                }
            }
            "vol" | "volume" => {
                if parts.len() > 1 {
                    if let Ok(val) = parts[1].parse::<f32>() {
                        session.char_cfg.audio.volume = val.clamp(0.0, 3.0);
                        println!("✔ 输出音量已设为: {:.2}", session.char_cfg.audio.volume);
                        let _ = session.play_synthesized();
                    } else {
                        println!("格式错误，示例: vol 1.3");
                    }
                } else {
                    println!("当前音量: {:.2}", session.char_cfg.audio.volume);
                }
            }
            "spk" => {
                if parts.len() > 1 {
                    if let Ok(val) = parts[1].parse::<u32>() {
                        session.char_cfg.audio.speaker_id = val;
                        println!("✔ Beatrice Speaker ID 已设为: {}", val);
                        let _ = session.init_or_update_beatrice(false);
                    }
                }
            }
            "v" | "voice" => {
                if parts.len() > 1 {
                    session.char_cfg.audio.tts_voice = parts[1].to_string();
                    println!("✔ Edge-TTS Voice 已设为: {}", parts[1]);
                    match session.synthesize_current().await {
                        Ok(_) => {
                            let _ = session.play_synthesized();
                        }
                        Err(e) => println!("\x1b[31m合成失败: {e}\x1b[0m"),
                    }
                } else {
                    println!("当前 Voice: {}", session.char_cfg.audio.tts_voice);
                }
            }
            "rate" => {
                if parts.len() > 1 {
                    session.char_cfg.audio.tts_rate = parts[1].to_string();
                    println!("✔ Edge-TTS Rate 已设为: {}", parts[1]);
                    match session.synthesize_current().await {
                        Ok(_) => {
                            let _ = session.play_synthesized();
                        }
                        Err(e) => println!("\x1b[31m合成失败: {e}\x1b[0m"),
                    }
                }
            }
            "tts_pitch" => {
                if parts.len() > 1 {
                    session.char_cfg.audio.tts_pitch = parts[1].to_string();
                    println!("✔ Edge-TTS Pitch 已设为: {}", parts[1]);
                    match session.synthesize_current().await {
                        Ok(_) => {
                            let _ = session.play_synthesized();
                        }
                        Err(e) => println!("\x1b[31m合成失败: {e}\x1b[0m"),
                    }
                }
            }
            "text" => {
                if parts.len() > 1 {
                    let new_text = line[4..].trim().to_string();
                    session.test_text = new_text;
                    println!("✔ 测试台词已更新为: \"{}\"", session.test_text);
                    match session.synthesize_current().await {
                        Ok(_) => {
                            let _ = session.play_synthesized();
                        }
                        Err(e) => println!("\x1b[31m合成失败: {e}\x1b[0m"),
                    }
                } else {
                    println!("当前测试台词: \"{}\"", session.test_text);
                }
            }
            "ref" => {
                if parts.len() > 1 {
                    let path_str = line[3..].trim();
                    let p = PathBuf::from(path_str);
                    if let Err(e) = session.set_reference_audio(p) {
                        println!("\x1b[31m设置参考音频失败: {e}\x1b[0m");
                    }
                } else {
                    let _ = session.play_reference();
                }
            }
            "voices" => {
                println!("\n常用推荐发音人:");
                println!("  [中文]");
                println!("   • zh-CN-XiaoyiNeural    (小艺，甜美温柔，适合多数日系角色转换)");
                println!("   • zh-CN-XiaoxiaoNeural  (晓晓，自然女声)");
                println!("   • zh-CN-Liaoning-XiaobeiNeural (辽宁小北，生动)");
                println!("  [日语]");
                println!("   • ja-JP-NanamiNeural    (七海，元气女声，与日系原声契合度极高)");
                println!("   • ja-JP-AoiNeural       (葵，清亮温柔)");
                println!("  使用 'voice <发音人名称>' 进行切换。\n");
            }
            "save" => {
                if let Err(e) = session.save_to_config() {
                    println!("\x1b[31m保存配置失败: {e}\x1b[0m");
                }
            }
            _ => {
                println!("未知指令: '{}'，输入 'help' 或 'h' 查看说明", line);
            }
        }
    }

    Ok(())
}
