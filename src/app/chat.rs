use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;

use crate::{
    ai::{
        AgentRunner, AgentStreamEvent, HermesClient, HermesRunner, LlmClient, StreamCueParser,
        StreamEvent, generate_system_prompt, mcp::McpManager, skills::SkillManager,
    },
    audio::{AudioPlayer, TtsEngine, VoiceChanger},
    character::CharacterManager,
    config::AppConfig,
    db::Database,
    model::ModelCatalog,
    renderer::Live2dRenderer,
};

use super::{
    ChatMessageData, ChatWindow, MainWindow, SubtitleWindow,
    chat_window::{ChatManager, refresh_messages_ui, refresh_sessions_ui},
    platform::{
        CHAT_OPEN_STATE, LAST_SUBTITLE_RECT, SUBTITLE_TARGET_HEIGHT, calculate_subtitle_height,
        force_update_subtitle_window_position,
    },
};

pub fn prepare_markdown_for_slint(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_code_block = false;

    for line in text.lines() {
        let trimmed = line.trim();

        // 1. Handle code blocks ```...
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }

        if in_code_block {
            if !trimmed.is_empty() {
                if trimmed.contains('`') {
                    result.push_str(line);
                } else {
                    result.push('`');
                    result.push_str(trimmed);
                    result.push('`');
                }
            }
            result.push('\n');
            continue;
        }

        // 2. Handle headings # Heading -> **Heading**
        if let Some(heading_content) = trimmed.strip_prefix('#') {
            let heading_text = heading_content.trim_start_matches('#').trim();
            if !heading_text.is_empty() {
                result.push_str("**");
                result.push_str(heading_text);
                result.push_str("**  \n");
                continue;
            }
        }

        // 3. Handle blockquotes > Quote -> ▍ Quote
        if let Some(quote_content) = trimmed.strip_prefix('>') {
            result.push_str("▍ ");
            result.push_str(quote_content.trim());
            result.push_str("  \n");
            continue;
        }

        if !trimmed.is_empty() {
            result.push_str(line);
            if !line.ends_with("  ") {
                result.push_str("  ");
            }
            result.push('\n');
        } else {
            result.push('\n');
        }
    }

    if !text.ends_with('\n') && result.ends_with('\n') {
        result.pop();
        if result.ends_with("  ") {
            result.pop();
            result.pop();
        }
    }

    // 4. Sanitize unsupported HTML tags (keep only <u> and <font>)
    sanitize_html_tags(&result)
}

fn sanitize_html_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' {
            let mut tag = String::new();
            let mut closed = false;
            while let Some(&c) = chars.peek() {
                chars.next();
                if c == '>' {
                    closed = true;
                    break;
                }
                tag.push(c);
                if tag.len() > 128 {
                    break;
                }
            }
            if closed {
                let trimmed = tag.trim().to_lowercase();
                if trimmed == "u" || trimmed == "/u" || trimmed.starts_with("font ") || trimmed == "/font" {
                    out.push('<');
                    out.push_str(&tag);
                    out.push('>');
                } else {
                    out.push_str("&lt;");
                    out.push_str(&tag);
                    out.push_str("&gt;");
                }
            } else {
                out.push_str("&lt;");
                out.push_str(&tag);
            }
        } else {
            out.push(ch);
        }
    }
    out
}

pub fn safe_styled_text(text: &str) -> slint::StyledText {
    let prepared = prepare_markdown_for_slint(text);
    slint::StyledText::from_markdown(&prepared)
        .unwrap_or_else(|_| slint::StyledText::from_plain_text(text))
}

pub fn set_subtitle_window_text(sub: &SubtitleWindow, text: &str) {
    sub.set_subtitle_text(safe_styled_text(text));
    sub.set_subtitle_raw_text(text.into());
}

struct AudioPlaybackItem {
    text: String,
    handle: tokio::task::JoinHandle<(Vec<f32>, u32, Vec<String>)>,
}

pub fn execute_chat_stream(
    prompt_text: String,
    session_id: String,
    sync_live2d: bool,
    config: Arc<Mutex<AppConfig>>,
    char_manager: Arc<Mutex<CharacterManager>>,
    db: Arc<Database>,
    model_catalog: Arc<Mutex<ModelCatalog>>,
    tokio_rt: Arc<tokio::runtime::Runtime>,
    tts_engine: Arc<TtsEngine>,
    voice_changer: Arc<Mutex<Option<VoiceChanger>>>,
    audio_player: Arc<Mutex<AudioPlayer>>,
    live2d_renderer: Arc<Mutex<Option<Live2dRenderer>>>,
    is_speaking: Arc<AtomicBool>,
    subtitle_window: slint::Weak<SubtitleWindow>,
    chat_window: Option<slint::Weak<ChatWindow>>,
    main_window: Option<slint::Weak<MainWindow>>,
) {
    let cfg = config.lock().unwrap().clone();
    let active_char = char_manager
        .lock()
        .unwrap()
        .get_active_character()
        .unwrap()
        .clone();

    // 1. Save user message to database with precomputed token count (Message-level caching)
    let active_model = active_char.ai.get_active_model();
    let user_token_count = crate::ai::count_tokens(&prompt_text, active_model);
    let _ = db.add_message(&session_id, "user", &prompt_text, user_token_count);

    // Auto title if session was default "新会话"
    if let Ok(Some(s)) = db.get_session(&session_id) {
        if s.title == "新会话" {
            let title: String = prompt_text.chars().take(14).collect();
            let _ = db.update_session_title(&session_id, &title);
        }
    }

    // 2. Update UI loading / generating states
    if let Some(ref win_weak) = main_window {
        if let Some(win) = win_weak.upgrade() {
            win.set_is_loading(true);
            win.set_chat_open(false);
        }
    }

    if let Some(ref cw_weak) = chat_window {
        if let Some(cw) = cw_weak.upgrade() {
            cw.set_is_generating(true);
            let cues_init = model_catalog.lock().unwrap().cues.clone();
            refresh_sessions_ui(&cw, &db, &active_char.id, &session_id);
            refresh_messages_ui(&cw, &db, &session_id, Some(&cues_init));

            // Append a streaming assistant placeholder message so token deltas update in real time
            let msgs_rc = cw.get_messages();
            let mut list: Vec<ChatMessageData> = Vec::new();
            for i in 0..msgs_rc.row_count() {
                if let Some(d) = msgs_rc.row_data(i) {
                    list.push(d);
                }
            }
            list.push(ChatMessageData {
                id: 0,
                role: "assistant".into(),
                content: safe_styled_text("..."),
                raw_text: "...".into(),
                created_at_str: "".into(),
                has_thought: false,
                thought_text: "".into(),
                has_tools: false,
                tools_summary: "".into(),
                tools_detail: "".into(),
            });
            cw.set_messages(ModelRc::new(VecModel::from(list)));
            cw.invoke_force_scroll_to_bottom();
        }
    }

    if sync_live2d {
        if let Ok(mut audio_p) = audio_player.lock() {
            audio_p.stop();
        }
        #[cfg(windows)]
        {
            CHAT_OPEN_STATE.store(false, Ordering::Relaxed);
            SUBTITLE_TARGET_HEIGHT.store(90, Ordering::Relaxed);
            if let Ok(mut last) = LAST_SUBTITLE_RECT.lock() {
                *last = None;
            }
        }
        if let Some(sub) = subtitle_window.upgrade() {
            set_subtitle_window_text(&sub, "...");
            sub.set_subtitle_visible(true);
            sub.set_subtitle_opacity(1.0);
            sub.window().request_redraw();
            force_update_subtitle_window_position(cfg.ui.subtitle_y_offset as i32);
        }
    }

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AgentStreamEvent>();
    let prompt_clone = prompt_text.clone();
    let sid_task = session_id.clone();
    let db_task = Arc::clone(&db);
    let char_task = active_char.clone();
    let catalog_task = Arc::clone(&model_catalog);
    let win_err = main_window.clone();
    let sub_err = subtitle_window.clone();
    let cw_err = chat_window.clone();
    let cfg_task = cfg.clone();

    // Spawn LLM / Agent call
    let llm_task = tokio_rt.spawn(async move {
        let session_info = db_task.get_session(&sid_task).ok().flatten();
        let protocol = session_info
            .as_ref()
            .map(|s| s.protocol.clone())
            .unwrap_or_else(|| char_task.ai.protocol.clone());

        let cues = catalog_task.lock().unwrap().cues.clone();
        let system_prompt = generate_system_prompt(&char_task.ai.system_prompt_prefix, &cues);

        let effective_agent_cfg = cfg_task.agent.merge_with(&char_task.agent);
        let skills_mgr = SkillManager::load_all("skills", Some(&char_task.id), "characters");

        let (result, completion_tokens): (anyhow::Result<(String, String)>, Option<u32>) =
            if protocol == "hermes" {
                Box::pin(async {
                    let mut mcp_mgr = McpManager::from_config(&effective_agent_cfg.mcp_servers);
                    mcp_mgr.ensure_all_connected().await;
                    let mcp_tools = mcp_mgr.list_all_tools();

                    let skills_list = skills_mgr.list_skills();
                    let client_context = crate::ai::skills::render_client_context(
                        &skills_list,
                        &effective_agent_cfg.builtin_tools,
                        &mcp_tools,
                    );

                    let hermes_client = HermesClient::new(&char_task.ai);
                    let last_resp_id = session_info
                        .as_ref()
                        .and_then(|s| s.last_response_id.clone());

                    let runner = HermesRunner::new(
                        hermes_client,
                        &effective_agent_cfg,
                        system_prompt,
                        sid_task.clone(),
                        client_context,
                    );

                    match runner
                        .run_turn(
                            &prompt_clone,
                            last_resp_id,
                            &skills_mgr,
                            &mut mcp_mgr,
                            event_tx.clone(),
                        )
                        .await
                    {
                        Ok((composite_resp, clean_resp, new_id)) => {
                            if let Some(ref n_id) = new_id {
                                let _ = db_task.update_session_response_id(&sid_task, Some(n_id));
                            }
                            (Ok((composite_resp, clean_resp)), None)
                        }
                        Err(e) => (Err(e), None),
                    }
                })
                .await
            } else if effective_agent_cfg.enabled {
                Box::pin(async {
                    let mut mcp_mgr = McpManager::from_config(&effective_agent_cfg.mcp_servers);
                    let agent_runner = AgentRunner::new(
                        &char_task.ai,
                        &effective_agent_cfg,
                        system_prompt,
                        &skills_mgr,
                    );
                    let history = db_task.get_messages(&sid_task).unwrap_or_default();
                    match agent_runner
                        .run_turn(
                            &history,
                            &prompt_clone,
                            &skills_mgr,
                            &mut mcp_mgr,
                            event_tx.clone(),
                        )
                        .await
                    {
                        Ok((full_resp, usage_tokens)) => {
                            let clean = crate::ai::cue_parser::strip_tool_xml(&full_resp);
                            (Ok((full_resp, clean)), usage_tokens)
                        }
                        Err(e) => (Err(e), None),
                    }
                })
                .await
            } else {
                Box::pin(async {
                    let llm_client = LlmClient::new(&char_task.ai, system_prompt);
                    let history = db_task.get_messages(&sid_task).unwrap_or_default();
                    let (raw_tok_tx, mut raw_tok_rx) = mpsc::unbounded_channel::<String>();
                    let event_tx_fwd = event_tx.clone();
                    tokio::spawn(async move {
                        while let Some(tok) = raw_tok_rx.recv().await {
                            let _ = event_tx_fwd.send(AgentStreamEvent::Token(tok));
                        }
                    });
                    match llm_client
                        .ask_stream_with_history(&history, &prompt_clone, raw_tok_tx)
                        .await
                    {
                        Ok((full_resp, usage_tokens)) => {
                            let clean = crate::ai::cue_parser::strip_tool_xml(&full_resp);
                            (Ok((full_resp, clean)), usage_tokens)
                        }
                        Err(e) => (Err(e), None),
                    }
                })
                .await
            };

        match result {
            Ok((composite_text, clean_final_text)) => {
                let clean_display_text =
                    crate::ai::cue_parser::strip_cues(&clean_final_text, Some(&cues));
                let assistant_token_count = if let Some(toks) = completion_tokens {
                    toks as i64
                } else {
                    let active_model = char_task.ai.get_active_model();
                    crate::ai::count_tokens(&clean_display_text, active_model)
                };
                let _ = db_task.add_message(
                    &sid_task,
                    "assistant",
                    &composite_text,
                    assistant_token_count,
                );
                true
            }
            Err(e) => {
                let err_msg = e.to_string();
                tracing::error!("LLM streaming request error: {err_msg}");
                let _ =
                    db_task.add_message(&sid_task, "assistant", &format!("请求失败: {err_msg}"), 0);
                let cues_err = cues.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ref w_weak) = win_err {
                        if let Some(win) = w_weak.upgrade() {
                            win.set_is_loading(false);
                            win.set_error_dialog_open(true);
                            win.set_error_dialog_title("AI 对话请求失败".into());
                            win.set_error_dialog_message(err_msg.into());
                            win.set_error_copy_btn_text("复制错误信息".into());
                        }
                    }
                    if let Some(sub) = sub_err.upgrade() {
                        set_subtitle_window_text(&sub, "[请求失败: 请查看错误提示]");
                        sub.window().request_redraw();
                    }
                    if let Some(ref cw_weak) = cw_err {
                        if let Some(cw) = cw_weak.upgrade() {
                            cw.set_is_generating(false);
                            cw.set_agent_status_text("".into());
                            refresh_messages_ui(&cw, &db_task, &sid_task, Some(&cues_err));
                        }
                    }
                });
                false
            }
        }
    });

    // Stream consumer & optional TTS / Live2D sync
    let catalog_stream = Arc::clone(&model_catalog);
    let tts_stream = Arc::clone(&tts_engine);
    let vc_stream = Arc::clone(&voice_changer);
    let player_stream = Arc::clone(&audio_player);
    let live2d_stream = Arc::clone(&live2d_renderer);
    let is_speaking_stream = Arc::clone(&is_speaking);
    let sub_stream = subtitle_window.clone();
    let win_stream = main_window.clone();
    let cw_stream = chat_window.clone();
    let cfg_stream = cfg.clone();
    let char_stream = active_char.clone();
    let db_stream = Arc::clone(&db);
    let sid_stream = session_id.clone();
    let effective_agent_for_stream = cfg.agent.merge_with(&active_char.agent);

    tokio_rt.spawn(async move {
        let cues = catalog_stream.lock().unwrap().cues.clone();
        let mut parser = StreamCueParser::new(cues);
        let tts_enabled = sync_live2d && char_stream.audio.tts_enabled;
        let _ = tts_enabled;

        let (audio_task_tx, mut audio_task_rx) = mpsc::unbounded_channel::<AudioPlaybackItem>();

        let player_worker = Arc::clone(&player_stream);
        let is_speaking_worker = Arc::clone(&is_speaking_stream);
        let cfg_audio_play = char_stream.audio.clone();
        let cfg_ui_play = cfg_stream.ui.clone();
        let sub_weak_worker = sub_stream.clone();
        let cw_weak_worker = cw_stream.clone();
        let sid_worker = sid_stream.clone();
        let catalog_worker = Arc::clone(&catalog_stream);
        let live2d_worker = Arc::clone(&live2d_stream);

        let audio_worker = tokio::spawn(async move {
            let mut accumulated_tts_subtitle = String::new();

            while let Some(item) = audio_task_rx.recv().await {
                let AudioPlaybackItem {
                    text: sentence_text,
                    handle,
                } = item;
                let (samples, sample_rate, _cues) = match handle.await {
                    Ok((samples, sample_rate, cues)) => (samples, sample_rate, cues),
                    Err(e) => {
                        tracing::error!("TTS task join error: {e}");
                        (Vec::new(), 24000, Vec::new())
                    }
                };

                if !_cues.is_empty() {
                    let catalog = catalog_worker.lock().unwrap();
                    let mut renderer_guard = live2d_worker.lock().unwrap();
                    for cue_name in _cues {
                        if let Some(action) = catalog.cues.get(&cue_name) {
                            if let Some(renderer) = renderer_guard.as_mut() {
                                if let Some(exp_path) = action.pick_expression() {
                                    let _ = renderer.play_expression(exp_path);
                                }
                                let motion_to_play = action.pick_motion().cloned().or_else(|| {
                                    action.pick_motion_group().and_then(|grp| {
                                        catalog.get_random_motion_from_group(grp)
                                    })
                                });
                                if let Some(motion_path) = motion_to_play {
                                    let _ = renderer.play_motion(motion_path);
                                }
                            }
                        }
                    }
                }

                if !samples.is_empty() {
                    is_speaking_worker.store(true, Ordering::Relaxed);
                    if let Ok(mut audio_p) = player_worker.lock() {
                        let _ = audio_p.play_or_append_samples(
                            samples,
                            sample_rate,
                            cfg_audio_play.volume,
                        );
                    }
                }

                if !sentence_text.trim().is_empty() {
                    if !accumulated_tts_subtitle.is_empty() {
                        let last_ch = accumulated_tts_subtitle.chars().last();
                        let first_ch = sentence_text.chars().next();
                        if let (Some(l), Some(f)) = (last_ch, first_ch) {
                            if l.is_ascii() && f.is_ascii() && !l.is_whitespace() {
                                accumulated_tts_subtitle.push(' ');
                            }
                        }
                    }
                    accumulated_tts_subtitle.push_str(&sentence_text);

                    let current_text = accumulated_tts_subtitle.clone();
                    let current_text_cw = current_text.clone();
                    let sub_weak_text = sub_weak_worker.clone();
                    let cw_weak_text = cw_weak_worker.clone();
                    let sid_target = sid_worker.clone();
                    let y_offset = cfg_ui_play.subtitle_y_offset as i32;
                    #[cfg(windows)]
                    {
                        let h = calculate_subtitle_height(
                            &current_text,
                            cfg_ui_play.subtitle_font_size as f32,
                            460.0,
                        );
                        SUBTITLE_TARGET_HEIGHT.store(h, Ordering::Relaxed);
                        if let Ok(mut last) = LAST_SUBTITLE_RECT.lock() {
                            *last = None;
                        }
                    }
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(sub) = sub_weak_text.upgrade() {
                            set_subtitle_window_text(&sub, &current_text);
                            sub.set_subtitle_visible(true);
                            sub.set_subtitle_opacity(1.0);
                            sub.window().request_redraw();
                            force_update_subtitle_window_position(y_offset);
                        }
                        if let Some(ref cw_weak) = cw_weak_text {
                            if let Some(cw) = cw_weak.upgrade() {
                                if cw.get_active_session_id().as_str() == sid_target {
                                    let msgs_rc = cw.get_messages();
                                    let count = msgs_rc.row_count();
                                    if count > 0 {
                                        if let Some(last_msg) = msgs_rc.row_data(count - 1) {
                                            if last_msg.role == "assistant" {
                                                let updated = ChatMessageData {
                                                    id: last_msg.id,
                                                    role: last_msg.role,
                                                    content: safe_styled_text(&current_text_cw),
                                                    raw_text: current_text_cw.into(),
                                                    created_at_str: last_msg.created_at_str,
                                                    has_thought: last_msg.has_thought,
                                                    thought_text: last_msg.thought_text,
                                                    has_tools: last_msg.has_tools,
                                                    tools_summary: last_msg.tools_summary,
                                                    tools_detail: last_msg.tools_detail,
                                                };
                                                msgs_rc.set_row_data(count - 1, updated);
                                                cw.invoke_scroll_to_bottom();
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    });
                }
            }

            while player_worker
                .lock()
                .map(|p| p.is_playing())
                .unwrap_or(false)
            {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            is_speaking_worker.store(false, Ordering::Relaxed);
        });

        let tts_semaphore = Arc::new(tokio::sync::Semaphore::new(1));
        let spawn_sentence_audio = {
            let tts_worker = Arc::clone(&tts_stream);
            let vc_worker = Arc::clone(&vc_stream);
            let cfg_audio = char_stream.audio.clone();
            let tts_sem = Arc::clone(&tts_semaphore);
            move |sentence: String,
                  cues: Vec<String>|
                  -> tokio::task::JoinHandle<(Vec<f32>, u32, Vec<String>)> {
                let tts = Arc::clone(&tts_worker);
                let vc = Arc::clone(&vc_worker);
                let cfg_audio = cfg_audio.clone();
                let sem = Arc::clone(&tts_sem);
                tokio::spawn(async move {
                    if sentence.trim().is_empty() {
                        return (Vec::new(), 24000, cues);
                    }
                    let _permit = sem.acquire_owned().await.ok();
                    let pcm_res = tts
                        .synthesize_to_pcm(
                            &sentence,
                            &cfg_audio.tts_voice,
                            &cfg_audio.tts_rate,
                            &cfg_audio.tts_pitch,
                            &cfg_audio.tts_volume,
                        )
                        .await;

                    match pcm_res {
                        Ok(pcm_24k) => {
                            if cfg_audio.beatrice_enabled {
                                let vc_clone = Arc::clone(&vc);
                                let converted = tokio::task::spawn_blocking(move || {
                                    let mut guard = vc_clone.lock().unwrap();
                                    if let Some(engine) = guard.as_mut() {
                                        engine.convert(&pcm_24k)
                                    } else {
                                        Err(anyhow::anyhow!("VoiceChanger not initialized"))
                                    }
                                })
                                .await;

                                match converted {
                                    Ok(Ok(samples_48k)) => (samples_48k, 48000, cues),
                                    Ok(Err(e)) => {
                                        tracing::error!("VoiceChanger convert error: {e}");
                                        (Vec::new(), 24000, cues)
                                    }
                                    Err(e) => {
                                        tracing::error!("VoiceChanger task join error: {e}");
                                        (Vec::new(), 24000, cues)
                                    }
                                }
                            } else {
                                (pcm_24k, 24000, cues)
                            }
                        }
                        Err(e) => {
                            tracing::error!("TTS synthesize error for sentence: {e}");
                            (Vec::new(), 24000, cues)
                        }
                    }
                })
            }
        };

        let mut accumulated_clean_text = String::new();
        let mut accumulated_thought = String::new();
        let mut accumulated_tools: Vec<(String, String)> = Vec::new();
        let mut pending_text = String::new();
        let mut pending_cues: Vec<String> = Vec::new();

        fn flush_pending_text(
            pending_text: &mut String,
            pending_cues: &mut Vec<String>,
            audio_task_tx: &mpsc::UnboundedSender<AudioPlaybackItem>,
            spawn_sentence_audio: &impl Fn(
                String,
                Vec<String>,
            )
                -> tokio::task::JoinHandle<(Vec<f32>, u32, Vec<String>)>,
        ) {
            let text = std::mem::take(pending_text);
            let cues = std::mem::take(pending_cues);
            if text.trim().is_empty() {
                *pending_cues = cues;
                return;
            }
            let handle = spawn_sentence_audio(text.clone(), cues.clone());
            let _ = audio_task_tx.send(AudioPlaybackItem { text, handle });
        }

        let mut last_ui_stream_time = std::time::Instant::now()
            .checked_sub(Duration::from_secs(1))
            .unwrap_or_else(std::time::Instant::now);

        while let Some(agent_event) = event_rx.recv().await {
            match agent_event {
                AgentStreamEvent::Token(token) => {
                    let events = parser.process_token(&token);
                    for event in events {
                        match event {
                            StreamEvent::Text(text) => {
                                if accumulated_clean_text.is_empty() {
                                    let trimmed = text.trim_start();
                                    if !trimmed.is_empty() {
                                        accumulated_clean_text.push_str(trimmed);
                                    }
                                } else {
                                    accumulated_clean_text.push_str(&text);
                                }
                            }
                            StreamEvent::Sentence(sentence) => {
                                if sync_live2d && tts_enabled {
                                    pending_text.push_str(&sentence);
                                    flush_pending_text(
                                        &mut pending_text,
                                        &mut pending_cues,
                                        &audio_task_tx,
                                        &spawn_sentence_audio,
                                    );
                                }
                            }
                            StreamEvent::Cue(cue_name) => {
                                if sync_live2d {
                                    pending_cues.push(cue_name);
                                }
                            }
                        }
                    }

                    if !tts_enabled {
                        // Update streaming message in ChatWindow (throttled to 35ms to avoid UI thread thrashing)
                        let now = std::time::Instant::now();
                        if now.duration_since(last_ui_stream_time) >= Duration::from_millis(35) {
                            last_ui_stream_time = now;
                            if let Some(ref cw_weak) = cw_stream {
                                let stream_text = accumulated_clean_text.clone();
                                let cw_up = cw_weak.clone();
                                let sid_target = sid_stream.clone();
                                let _ = slint::invoke_from_event_loop(move || {
                                    if let Some(cw) = cw_up.upgrade() {
                                        if cw.get_active_session_id().as_str() == sid_target {
                                            let msgs_rc = cw.get_messages();
                                            let count = msgs_rc.row_count();
                                            if count > 0 {
                                                if let Some(last_msg) = msgs_rc.row_data(count - 1) {
                                                    if last_msg.role == "assistant" {
                                                        let updated = ChatMessageData {
                                                            id: last_msg.id,
                                                            role: last_msg.role,
                                                            content: safe_styled_text(&stream_text),
                                                            raw_text: stream_text.into(),
                                                            created_at_str: last_msg.created_at_str,
                                                            has_thought: last_msg.has_thought,
                                                            thought_text: last_msg.thought_text,
                                                            has_tools: last_msg.has_tools,
                                                            tools_summary: last_msg.tools_summary,
                                                            tools_detail: last_msg.tools_detail,
                                                        };
                                                        msgs_rc.set_row_data(count - 1, updated);
                                                        cw.invoke_scroll_to_bottom();
                                                        if cw.get_agent_status_text() == "正在思考..." {
                                                            cw.set_agent_status_text("".into());
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                });
                            }
                        }
                    }
                }
                AgentStreamEvent::Thought(chunk) => {
                    let sub_weak_th = sub_stream.clone();
                    let cw_weak_th = cw_stream.clone();
                    accumulated_thought.push_str(&chunk);
                    let thought_display = accumulated_thought.clone();
                    let sid_target = sid_stream.clone();

                    if effective_agent_for_stream.live2d_thinking_enabled {
                        if let Ok(catalog) = catalog_stream.lock() {
                            let target_cue = &effective_agent_for_stream.thinking_cue;
                            let action_opt = catalog
                                .cues
                                .get(target_cue)
                                .or_else(|| catalog.cues.get("thinking"))
                                .or_else(|| catalog.cues.get("awkward"));
                            if let Some(action) = action_opt {
                                if let Ok(mut renderer_guard) = live2d_stream.lock() {
                                    if let Some(renderer) = renderer_guard.as_mut() {
                                        if let Some(exp_path) = action.pick_expression() {
                                            let _ = renderer.play_expression(exp_path);
                                        }
                                    }
                                }
                            }
                        }
                    }

                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(cw) = cw_weak_th.as_ref().and_then(|w| w.upgrade()) {
                            if cw.get_active_session_id().as_str() == sid_target {
                                let msgs_rc = cw.get_messages();
                                let count = msgs_rc.row_count();
                                if count > 0 {
                                    if let Some(last_msg) = msgs_rc.row_data(count - 1) {
                                        if last_msg.role == "assistant" {
                                            let is_placeholder = last_msg.raw_text == "...";
                                            let updated = ChatMessageData {
                                                id: last_msg.id,
                                                role: last_msg.role,
                                                content: if is_placeholder { safe_styled_text("") } else { last_msg.content },
                                                raw_text: if is_placeholder { "".into() } else { last_msg.raw_text },
                                                created_at_str: last_msg.created_at_str,
                                                has_thought: true,
                                                thought_text: thought_display.into(),
                                                has_tools: last_msg.has_tools,
                                                tools_summary: last_msg.tools_summary,
                                                tools_detail: last_msg.tools_detail,
                                            };
                                            msgs_rc.set_row_data(count - 1, updated);
                                            cw.invoke_scroll_to_bottom();
                                        }
                                    }
                                }
                            }
                            cw.set_agent_status_text("正在思考...".into());
                        }
                        if effective_agent_for_stream.subtitle_thinking_status {
                            if let Some(sub) = sub_weak_th.upgrade() {
                                set_subtitle_window_text(&sub, "正在思考...");
                                sub.set_subtitle_visible(true);
                                sub.window().request_redraw();
                            }
                        }
                    });
                }
                AgentStreamEvent::Status(status) => {
                    let sub_weak_status = sub_stream.clone();
                    let cw_weak_status = cw_stream.clone();
                    let status_clone = status.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(cw) = cw_weak_status.as_ref().and_then(|w| w.upgrade()) {
                            cw.set_agent_status_text(status_clone.clone().into());
                        }
                        if !status_clone.is_empty()
                            && effective_agent_for_stream.subtitle_tool_call_status
                        {
                            if let Some(sub) = sub_weak_status.upgrade() {
                                set_subtitle_window_text(&sub, &status_clone);
                                sub.set_subtitle_visible(true);
                                sub.window().request_redraw();
                            }
                        }
                    });
                }
                AgentStreamEvent::ToolCallStart { name, input, .. } => {
                    accumulated_tools.push((name.clone(), input.clone()));
                    let status_str = format!("正在执行: {name}...");
                    let sub_weak_tc = sub_stream.clone();
                    let cw_weak_tc = cw_stream.clone();
                    let status_clone = status_str.clone();
                    let sid_target = sid_stream.clone();
                    let tool_name = name.clone();
                    let tool_input = input.clone();
                    let total_tools = accumulated_tools.len();

                    if effective_agent_for_stream.live2d_tool_cue_enabled {
                        if let Ok(catalog) = catalog_stream.lock() {
                            let target_cue = &effective_agent_for_stream.tool_cue;
                            let action_opt = catalog
                                .cues
                                .get(target_cue)
                                .or_else(|| catalog.cues.get("nod"))
                                .or_else(|| catalog.cues.get("earnest"))
                                .or_else(|| catalog.cues.get("awkward"));
                            if let Some(action) = action_opt {
                                if let Ok(mut renderer_guard) = live2d_stream.lock() {
                                    if let Some(renderer) = renderer_guard.as_mut() {
                                        if let Some(exp_path) = action.pick_expression() {
                                            let _ = renderer.play_expression(exp_path);
                                        }
                                    }
                                }
                            }
                        }
                    }

                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(cw) = cw_weak_tc.as_ref().and_then(|w| w.upgrade()) {
                            if cw.get_active_session_id().as_str() == sid_target {
                                let msgs_rc = cw.get_messages();
                                let count = msgs_rc.row_count();
                                if count > 0 {
                                    if let Some(last_msg) = msgs_rc.row_data(count - 1) {
                                        if last_msg.role == "assistant" {
                                            let summary = if total_tools == 1 {
                                                format!("正在执行工具: {tool_name}")
                                            } else {
                                                format!("正在执行第 {total_tools} 项工具: {tool_name}")
                                            };
                                            let detail_part = format!("工具: {tool_name}\n参数: {tool_input}");
                                            let new_detail = if last_msg.tools_detail.is_empty() {
                                                detail_part
                                            } else {
                                                format!("{}\n--------------------\n{}", last_msg.tools_detail, detail_part)
                                            };
                                            let is_placeholder = last_msg.raw_text == "...";
                                            let updated = ChatMessageData {
                                                id: last_msg.id,
                                                role: last_msg.role,
                                                content: if is_placeholder { safe_styled_text("") } else { last_msg.content },
                                                raw_text: if is_placeholder { "".into() } else { last_msg.raw_text },
                                                created_at_str: last_msg.created_at_str,
                                                has_thought: last_msg.has_thought,
                                                thought_text: last_msg.thought_text,
                                                has_tools: true,
                                                tools_summary: summary.into(),
                                                tools_detail: new_detail.into(),
                                            };
                                            msgs_rc.set_row_data(count - 1, updated);
                                            cw.invoke_scroll_to_bottom();
                                        }
                                    }
                                }
                            }
                            cw.set_agent_status_text(status_clone.into());
                        }
                        if effective_agent_for_stream.subtitle_tool_call_status {
                            if let Some(sub) = sub_weak_tc.upgrade() {
                                set_subtitle_window_text(&sub, &status_str);
                                sub.set_subtitle_visible(true);
                                sub.window().request_redraw();
                            }
                        }
                    });
                }
                AgentStreamEvent::ToolCallDone { name, .. } => {
                    let cw_weak_done = cw_stream.clone();
                    let sid_target = sid_stream.clone();
                    let total_tools = accumulated_tools.len();
                    let done_name = name.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(cw) = cw_weak_done.as_ref().and_then(|w| w.upgrade()) {
                            if cw.get_active_session_id().as_str() == sid_target {
                                let msgs_rc = cw.get_messages();
                                let count = msgs_rc.row_count();
                                if count > 0 {
                                    if let Some(last_msg) = msgs_rc.row_data(count - 1) {
                                        if last_msg.role == "assistant" {
                                            let summary = if total_tools == 1 {
                                                format!("已执行工具: {done_name}")
                                            } else {
                                                format!("已执行 {total_tools} 项工具调用")
                                            };
                                            let updated = ChatMessageData {
                                                id: last_msg.id,
                                                role: last_msg.role,
                                                content: last_msg.content,
                                                raw_text: last_msg.raw_text,
                                                created_at_str: last_msg.created_at_str,
                                                has_thought: last_msg.has_thought,
                                                thought_text: last_msg.thought_text,
                                                has_tools: true,
                                                tools_summary: summary.into(),
                                                tools_detail: last_msg.tools_detail,
                                            };
                                            msgs_rc.set_row_data(count - 1, updated);
                                        }
                                    }
                                }
                            }
                            cw.set_agent_status_text("".into());
                        }
                    });
                }
                AgentStreamEvent::ResponseId(_) | AgentStreamEvent::ToolCallProgress { .. } => {}
                AgentStreamEvent::Cancelled => {
                    let cw_weak_cancel = cw_stream.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(cw) = cw_weak_cancel.as_ref().and_then(|w| w.upgrade()) {
                            cw.set_agent_status_text("[已取消]".into());
                        }
                    });
                }
                AgentStreamEvent::Failed(err) => {
                    let cw_weak_fail = cw_stream.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(cw) = cw_weak_fail.as_ref().and_then(|w| w.upgrade()) {
                            cw.set_agent_status_text(format!("[失败: {err}]").into());
                        }
                    });
                }
            }
        }

        let remaining = parser.finish();
        for event in remaining {
            match event {
                StreamEvent::Text(text) => {
                    accumulated_clean_text.push_str(&text);
                }
                StreamEvent::Sentence(sentence) => {
                    if sync_live2d && tts_enabled {
                        pending_text.push_str(&sentence);
                        flush_pending_text(
                            &mut pending_text,
                            &mut pending_cues,
                            &audio_task_tx,
                            &spawn_sentence_audio,
                        );
                    }
                }
                StreamEvent::Cue(cue_name) => {
                    if sync_live2d {
                        pending_cues.push(cue_name);
                    }
                }
            }
        }

        if sync_live2d && tts_enabled {
            flush_pending_text(
                &mut pending_text,
                &mut pending_cues,
                &audio_task_tx,
                &spawn_sentence_audio,
            );
        }

        if !tts_enabled {
            if let Some(ref cw_weak) = cw_stream {
                let stream_text = accumulated_clean_text.trim().to_string();
                let cw_up = cw_weak.clone();
                let sid_target = sid_stream.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(cw) = cw_up.upgrade() {
                        if cw.get_active_session_id().as_str() == sid_target {
                            let msgs_rc = cw.get_messages();
                            let count = msgs_rc.row_count();
                            if count > 0 {
                                if let Some(last_msg) = msgs_rc.row_data(count - 1) {
                                    if last_msg.role == "assistant" {
                                        let updated = ChatMessageData {
                                            id: last_msg.id,
                                            role: last_msg.role,
                                            content: safe_styled_text(&stream_text),
                                            raw_text: stream_text.into(),
                                            created_at_str: last_msg.created_at_str,
                                            has_thought: last_msg.has_thought,
                                            thought_text: last_msg.thought_text,
                                            has_tools: last_msg.has_tools,
                                            tools_summary: last_msg.tools_summary,
                                            tools_detail: last_msg.tools_detail,
                                        };
                                        msgs_rc.set_row_data(count - 1, updated);
                                        cw.invoke_scroll_to_bottom();
                                    }
                                }
                            }
                        }
                    }
                });
            }
        }

        if sync_live2d {
            drop(audio_task_tx);
            let _ = audio_worker.await;
        } else {
            drop(audio_task_tx);
        }

        let llm_ok = llm_task.await.unwrap_or(false);

        if llm_ok {
            let cues_refresh = catalog_stream.lock().unwrap().cues.clone();
            if let Some(ref cw_weak) = cw_stream {
                let cw_up = cw_weak.clone();
                let db_c = Arc::clone(&db_stream);
                let sid_c = sid_stream.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(cw) = cw_up.upgrade() {
                        cw.set_is_generating(false);
                        cw.set_agent_status_text("".into());
                        if cw.get_active_session_id().as_str() == sid_c {
                            refresh_messages_ui(&cw, &db_c, &sid_c, Some(&cues_refresh));
                        }
                    }
                });
            }

            if let Some(ref w_weak) = win_stream {
                let win_up = w_weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = win_up.upgrade() {
                        win.set_is_loading(false);
                    }
                });
            }
        }
    });
}

pub fn setup_chat_callbacks(
    window: &MainWindow,
    subtitle_window: &SubtitleWindow,
    chat_window: &ChatWindow,
    chat_manager: Arc<ChatManager>,
    config: Arc<Mutex<AppConfig>>,
    char_manager: Arc<Mutex<CharacterManager>>,
    model_catalog: Arc<Mutex<ModelCatalog>>,
    tokio_rt: Arc<tokio::runtime::Runtime>,
    tts_engine: Arc<TtsEngine>,
    voice_changer: Arc<Mutex<Option<VoiceChanger>>>,
    audio_player: Arc<Mutex<AudioPlayer>>,
    live2d_renderer: Arc<Mutex<Option<Live2dRenderer>>>,
    is_speaking: Arc<AtomicBool>,
) {
    let window_weak = window.as_weak();
    let sub_weak = subtitle_window.as_weak();
    let chat_win_weak = chat_window.as_weak();

    let cfg_ref = Arc::clone(&config);
    let char_mgr_ref = Arc::clone(&char_manager);
    let mgr = Arc::clone(&chat_manager);
    let catalog = Arc::clone(&model_catalog);
    let rt = Arc::clone(&tokio_rt);
    let tts = Arc::clone(&tts_engine);
    let vc = Arc::clone(&voice_changer);
    let player = Arc::clone(&audio_player);
    let live2d = Arc::clone(&live2d_renderer);
    let is_spk = Arc::clone(&is_speaking);

    window.on_prompt_submitted(move |prompt| {
        let prompt_text = prompt.to_string();
        let session_id = mgr.get_active_session_id();
        execute_chat_stream(
            prompt_text,
            session_id,
            true, // Always sync live2d & TTS when submitted from desk pet
            Arc::clone(&cfg_ref),
            Arc::clone(&char_mgr_ref),
            Arc::clone(&mgr.db),
            Arc::clone(&catalog),
            Arc::clone(&rt),
            Arc::clone(&tts),
            Arc::clone(&vc),
            Arc::clone(&player),
            Arc::clone(&live2d),
            Arc::clone(&is_spk),
            sub_weak.clone(),
            Some(chat_win_weak.clone()),
            Some(window_weak.clone()),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_subtitle_window_text_markdown_sync() {
        let sub = SubtitleWindow::new().expect("Failed to initialize SubtitleWindow");
        let sample_md = "**丛雨**: *你好*，[Slint](https://slint.dev)！";
        set_subtitle_window_text(&sub, sample_md);

        assert_eq!(sub.get_subtitle_raw_text(), sample_md);
        assert_eq!(sub.get_subtitle_text(), safe_styled_text(sample_md));

        // Test clearing subtitle
        set_subtitle_window_text(&sub, "");
        assert_eq!(sub.get_subtitle_raw_text(), "");
        assert_eq!(sub.get_subtitle_text(), safe_styled_text(""));

        let test_cases = vec![
            ("Headings", "# Heading 1\n## Heading 2"),
            ("Blockquote", "> This is a quote"),
            ("Code block", "```markdown\n**粗体**\n```"),
            ("HTML tags", "<think>reasoning</think>Hello <u>world</u>!"),
            ("Inline code", "`code`"),
            ("User exact case", "用两侧双星号包住文字：\n\n```markdown\n**粗体**\n```\n\n或两侧双下划线：\n\n```markdown\n__粗体__\n```\n\n渲染后是：**粗体**"),
        ];
        for (name, tc) in test_cases {
            let styled = safe_styled_text(tc);
            assert_ne!(styled, slint::StyledText::default(), "Failed for case: {name}");
        }
    }
}
