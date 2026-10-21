use slint::{ComponentHandle, ModelRc, VecModel};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use crate::{
    audio::{AudioPlayer, TtsEngine, VoiceChanger},
    character::CharacterManager,
    config::AppConfig,
    db::Database,
    model::{CueRegistry, ModelCatalog},
    renderer::Live2dRenderer,
};

use super::{
    ChatMessageData, ChatWindow, SessionItem, SubtitleWindow,
    chat::{execute_chat_stream, safe_styled_text},
};

pub struct ChatManager {
    pub db: Arc<Database>,
    pub active_character_id: Arc<Mutex<String>>,
    pub active_session_id: Arc<Mutex<String>>,
    pub mute_sync: Arc<AtomicBool>,
}

impl ChatManager {
    pub fn new(
        db: Arc<Database>,
        active_character_id: &str,
        default_protocol: &str,
    ) -> anyhow::Result<Self> {
        let initial_session =
            db.get_or_create_default_session(active_character_id, default_protocol)?;
        Ok(Self {
            db,
            active_character_id: Arc::new(Mutex::new(active_character_id.to_string())),
            active_session_id: Arc::new(Mutex::new(initial_session.id)),
            mute_sync: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn get_active_character_id(&self) -> String {
        self.active_character_id.lock().unwrap().clone()
    }

    pub fn set_active_character_id(&self, id: String) {
        *self.active_character_id.lock().unwrap() = id;
    }

    pub fn get_active_session_id(&self) -> String {
        self.active_session_id.lock().unwrap().clone()
    }

    pub fn set_active_session_id(&self, id: String) {
        *self.active_session_id.lock().unwrap() = id;
    }

    pub fn switch_character(
        &self,
        new_char_id: &str,
        default_protocol: &str,
    ) -> anyhow::Result<()> {
        let session = self
            .db
            .get_or_create_default_session(new_char_id, default_protocol)?;
        *self.active_character_id.lock().unwrap() = new_char_id.to_string();
        *self.active_session_id.lock().unwrap() = session.id;
        Ok(())
    }
}

pub fn refresh_sessions_ui(
    chat_win: &ChatWindow,
    db: &Database,
    character_id: &str,
    active_session_id: &str,
) {
    if let Ok(sessions) = db.list_sessions(character_id) {
        let mut items = Vec::new();
        let mut found_title = "新会话".to_string();
        let mut found_protocol = "hermes".to_string();

        for s in sessions {
            let is_act = s.id == active_session_id;
            if is_act {
                found_title = s.title.clone();
                found_protocol = s.protocol.clone();
            }
            items.push(SessionItem {
                id: s.id.into(),
                title: s.title.into(),
                protocol: s.protocol.into(),
                updated_at: format!("{}", s.updated_at).into(),
                is_active: is_act,
            });
        }

        chat_win.set_sessions(ModelRc::new(VecModel::from(items)));
        chat_win.set_active_session_id(active_session_id.into());
        chat_win.set_active_session_title(found_title.into());
        chat_win.set_active_protocol(found_protocol.into());
    }
}

pub fn refresh_messages_ui(
    chat_win: &ChatWindow,
    db: &Database,
    session_id: &str,
    cues: Option<&CueRegistry>,
) {
    if let Ok(msgs) = db.get_messages(session_id) {
        let items: Vec<ChatMessageData> = msgs
            .into_iter()
            .map(|m| {
                if m.role == "assistant" {
                    let parts = crate::ai::cue_parser::extract_message_parts(&m.content);
                    let clean = crate::ai::cue_parser::strip_cues(&parts.clean_text, cues);
                    ChatMessageData {
                        id: m.id as i32,
                        role: m.role.into(),
                        content: safe_styled_text(&clean),
                        raw_text: clean.into(),
                        created_at_str: format!("{}", m.created_at).into(),
                        has_thought: parts.has_thought,
                        thought_text: parts.thought.into(),
                        has_tools: parts.has_tools,
                        tools_summary: parts.tools_summary.into(),
                        tools_detail: parts.tools_detail.into(),
                    }
                } else {
                    ChatMessageData {
                        id: m.id as i32,
                        role: m.role.into(),
                        content: safe_styled_text(&m.content),
                        raw_text: m.content.into(),
                        created_at_str: format!("{}", m.created_at).into(),
                        has_thought: false,
                        thought_text: "".into(),
                        has_tools: false,
                        tools_summary: "".into(),
                        tools_detail: "".into(),
                    }
                }
            })
            .collect();
        chat_win.set_messages(ModelRc::new(VecModel::from(items)));
        chat_win.invoke_force_scroll_to_bottom();
    } else {
        chat_win.set_messages(ModelRc::new(VecModel::from(Vec::new())));
        chat_win.invoke_force_scroll_to_bottom();
    }
}

pub fn setup_chat_window_callbacks(
    chat_win: &ChatWindow,
    manager: Arc<ChatManager>,
    config: Arc<Mutex<AppConfig>>,
    char_manager: Arc<Mutex<CharacterManager>>,
    model_catalog: Arc<Mutex<ModelCatalog>>,
    tokio_rt: Arc<tokio::runtime::Runtime>,
    tts_engine: Arc<TtsEngine>,
    voice_changer: Arc<Mutex<Option<VoiceChanger>>>,
    audio_player: Arc<Mutex<AudioPlayer>>,
    live2d_renderer: Arc<Mutex<Option<Live2dRenderer>>>,
    subtitle_window: &SubtitleWindow,
    is_speaking: Arc<AtomicBool>,
    on_switch_character: Arc<dyn Fn(&str) + Send + Sync + 'static>,
) {
    let active_char_id = manager.get_active_character_id();
    let active_id = manager.get_active_session_id();
    let initial_cues = model_catalog.lock().ok().map(|c| c.cues.clone());
    refresh_sessions_ui(chat_win, &manager.db, &active_char_id, &active_id);
    refresh_messages_ui(chat_win, &manager.db, &active_id, initial_cues.as_ref());

    // 0. Character Switch from Chat Header
    {
        let switch_cb = Arc::clone(&on_switch_character);
        chat_win.on_switch_character(move |new_char_id| {
            switch_cb(&new_char_id.to_string());
        });
    }

    // 1. New Session
    {
        let chat_win_weak = chat_win.as_weak();
        let mgr = Arc::clone(&manager);
        let char_mgr_ref = Arc::clone(&char_manager);
        let cat_new = Arc::clone(&model_catalog);
        chat_win.on_new_session(move || {
            let active_char_id = mgr.get_active_character_id();
            let protocol = char_mgr_ref
                .lock()
                .unwrap()
                .get_character(&active_char_id)
                .map(|c| c.ai.protocol.clone())
                .unwrap_or_else(|| "hermes".to_string());
            if let Ok(new_sess) = mgr.db.create_session(&active_char_id, "新会话", &protocol) {
                mgr.set_active_session_id(new_sess.id.clone());
                if let Some(cw) = chat_win_weak.upgrade() {
                    let cues_opt = cat_new.lock().ok().map(|c| c.cues.clone());
                    refresh_sessions_ui(&cw, &mgr.db, &active_char_id, &new_sess.id);
                    refresh_messages_ui(&cw, &mgr.db, &new_sess.id, cues_opt.as_ref());
                }
            }
        });
    }

    // 2. Select Session
    {
        let chat_win_weak = chat_win.as_weak();
        let mgr = Arc::clone(&manager);
        let cat_sel = Arc::clone(&model_catalog);
        chat_win.on_select_session(move |sess_id| {
            let sid = sess_id.to_string();
            let active_char_id = mgr.get_active_character_id();
            mgr.set_active_session_id(sid.clone());
            if let Some(cw) = chat_win_weak.upgrade() {
                let cues_opt = cat_sel.lock().ok().map(|c| c.cues.clone());
                refresh_sessions_ui(&cw, &mgr.db, &active_char_id, &sid);
                refresh_messages_ui(&cw, &mgr.db, &sid, cues_opt.as_ref());
            }
        });
    }

    // 3. Delete Session
    {
        let chat_win_weak = chat_win.as_weak();
        let mgr = Arc::clone(&manager);
        let char_mgr_ref = Arc::clone(&char_manager);
        let cat_del = Arc::clone(&model_catalog);
        chat_win.on_delete_session(move |sess_id| {
            let sid = sess_id.to_string();
            let active_char_id = mgr.get_active_character_id();
            let _ = mgr.db.delete_session(&sid);
            let active = mgr.get_active_session_id();
            let new_active = if active == sid {
                let default_protocol = char_mgr_ref
                    .lock()
                    .unwrap()
                    .get_character(&active_char_id)
                    .map(|c| c.ai.protocol.clone())
                    .unwrap_or_else(|| "hermes".to_string());
                mgr.db
                    .get_or_create_default_session(&active_char_id, &default_protocol)
                    .map(|s| s.id)
                    .unwrap_or_default()
            } else {
                active
            };
            mgr.set_active_session_id(new_active.clone());
            if let Some(cw) = chat_win_weak.upgrade() {
                let cues_opt = cat_del.lock().ok().map(|c| c.cues.clone());
                refresh_sessions_ui(&cw, &mgr.db, &active_char_id, &new_active);
                refresh_messages_ui(&cw, &mgr.db, &new_active, cues_opt.as_ref());
            }
        });
    }

    // 4. Rename Session
    {
        let chat_win_weak = chat_win.as_weak();
        let mgr = Arc::clone(&manager);
        chat_win.on_rename_session(move |sess_id, new_title| {
            let sid = sess_id.to_string();
            let title = new_title.to_string();
            let active_char_id = mgr.get_active_character_id();
            let _ = mgr.db.update_session_title(&sid, &title);
            if let Some(cw) = chat_win_weak.upgrade() {
                refresh_sessions_ui(&cw, &mgr.db, &active_char_id, &mgr.get_active_session_id());
            }
        });
    }

    // 5. Clear Session
    {
        let chat_win_weak = chat_win.as_weak();
        let mgr = Arc::clone(&manager);
        let cat_clr = Arc::clone(&model_catalog);
        chat_win.on_clear_session(move |sess_id| {
            let sid = sess_id.to_string();
            let _ = mgr.db.clear_messages(&sid);
            if let Some(cw) = chat_win_weak.upgrade() {
                let cues_opt = cat_clr.lock().ok().map(|c| c.cues.clone());
                refresh_messages_ui(&cw, &mgr.db, &sid, cues_opt.as_ref());
            }
        });
    }

    // 6. Toggle Sidebar
    chat_win.on_toggle_sidebar(|| {});

    // 7. Copy to Clipboard
    chat_win.on_copy_to_clipboard(move |text| {
        crate::app::platform::set_clipboard_text(&text);
    });

    // 8. Send Message
    {
        let chat_win_weak = chat_win.as_weak();
        let sub_weak = subtitle_window.as_weak();
        let mgr = Arc::clone(&manager);
        let cfg_ref = Arc::clone(&config);
        let char_mgr_ref = Arc::clone(&char_manager);
        let catalog = Arc::clone(&model_catalog);
        let rt = Arc::clone(&tokio_rt);
        let tts = Arc::clone(&tts_engine);
        let vc = Arc::clone(&voice_changer);
        let player = Arc::clone(&audio_player);
        let live2d = Arc::clone(&live2d_renderer);
        let is_spk = Arc::clone(&is_speaking);

        chat_win.on_send_message(move |prompt| {
            let prompt_text = prompt.to_string();
            if prompt_text.trim().is_empty() {
                return;
            }

            let sync = !mgr.mute_sync.load(Ordering::Relaxed);
            let session_id = mgr.get_active_session_id();

            execute_chat_stream(
                prompt_text,
                session_id,
                sync,
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
                None,
            );
        });
    }
}

