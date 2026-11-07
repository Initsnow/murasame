use std::sync::{
    Arc, Mutex,
    atomic::AtomicBool,
};
use slint::ComponentHandle;
use crate::{
    audio::{AudioPlayer, TtsEngine, VoiceChanger},
    character::CharacterManager,
    config::AppConfig,
    model::ModelCatalog,
    renderer::Live2dRenderer,
};
use crate::app::{
    ChatWindow, MainWindow, SubtitleWindow,
    chat_window::ChatManager,
};
use super::stream::execute_chat_stream;

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

