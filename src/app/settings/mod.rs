pub mod populate;
pub mod voice;
pub mod llm;
pub mod agent;
pub mod live2d;
pub mod character;
pub mod save;

pub use populate::{
    build_mcp_server_items, populate_catalog_lists, populate_character_settings,
    populate_global_settings, refresh_mcp_servers_ui, refresh_skills_ui,
};

use std::sync::{Arc, Mutex};
use crate::{
    audio::{AudioPlayer, TtsEngine, VoiceChanger},
    character::CharacterManager,
    config::AppConfig,
    model::ModelCatalog,
    renderer::Live2dRenderer,
};
use crate::app::{
    ChatWindow, MainWindow, MurasameTray, SettingsWindow, SubtitleWindow,
};

pub fn setup_settings_callbacks(
    window: &MainWindow,
    tray: &MurasameTray,
    settings_window: &SettingsWindow,
    subtitle_window: &SubtitleWindow,
    chat_window: &ChatWindow,
    config: Arc<Mutex<AppConfig>>,
    char_manager: Arc<Mutex<CharacterManager>>,
    model_catalog: Arc<Mutex<ModelCatalog>>,
    tokio_rt: Arc<tokio::runtime::Runtime>,
    tts_engine: Arc<TtsEngine>,
    voice_changer: Arc<Mutex<Option<VoiceChanger>>>,
    audio_player: Arc<Mutex<AudioPlayer>>,
    live2d_renderer: Arc<Mutex<Option<Live2dRenderer>>>,
    on_switch_character: Arc<dyn Fn(&str) + Send + Sync + 'static>,
) {
    voice::setup(
        settings_window,
        &config,
        &tts_engine,
        &voice_changer,
        &audio_player,
        &tokio_rt,
    );

    llm::setup(
        settings_window,
        &tokio_rt,
    );

    agent::setup(
        settings_window,
        &config,
        &char_manager,
    );

    live2d::setup(
        settings_window,
        &config,
        &char_manager,
        &model_catalog,
        &live2d_renderer,
    );

    character::setup(
        settings_window,
        window,
        tray,
        chat_window,
        &config,
        &char_manager,
        &model_catalog,
        &on_switch_character,
    );

    save::setup(
        settings_window,
        window,
        subtitle_window,
        chat_window,
        &config,
        &char_manager,
        &model_catalog,
        &voice_changer,
        &live2d_renderer,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use crate::config::McpServerConfig;

    #[test]
    fn test_refresh_mcp_servers_ui() {
        let mut servers = HashMap::new();
        servers.insert(
            "filesystem".to_string(),
            McpServerConfig {
                enabled: true,
                transport: "stdio".to_string(),
                command: Some("npx".to_string()),
                args: vec!["-y".to_string(), "@modelcontextprotocol/server-filesystem".to_string()],
                env: HashMap::new(),
                url: None,
            },
        );
        servers.insert(
            "remote_mcp".to_string(),
            McpServerConfig {
                enabled: false,
                transport: "sse".to_string(),
                command: None,
                args: Vec::new(),
                env: HashMap::new(),
                url: Some("http://localhost:8000/sse".to_string()),
            },
        );

        let items = build_mcp_server_items(&servers);
        assert_eq!(items.len(), 2);

        let first = &items[0];
        assert_eq!(first.name, "filesystem");
        assert_eq!(first.transport, "stdio");
        assert!(first.detail.contains("npx"));
        assert!(first.enabled);

        let second = &items[1];
        assert_eq!(second.name, "remote_mcp");
        assert_eq!(second.transport, "sse");
        assert_eq!(second.detail, "http://localhost:8000/sse");
        assert!(!second.enabled);
    }
}

