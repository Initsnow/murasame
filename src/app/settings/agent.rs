use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use slint::ComponentHandle;
use crate::{
    character::CharacterManager,
    config::AppConfig,
};
use crate::app::SettingsWindow;
use super::populate::{refresh_mcp_servers_ui, refresh_skills_ui};

pub fn setup(
    settings_window: &SettingsWindow,
    config: &Arc<Mutex<AppConfig>>,
    char_manager: &Arc<Mutex<CharacterManager>>,
) {
    let sw_weak_skills = settings_window.as_weak();
    settings_window.on_reload_skills_clicked(move || {
        if let Some(sw) = sw_weak_skills.upgrade() {
            let cid = sw.get_editing_char_id().to_string();
            let app_root = crate::paths::app_root_dir();
            let mgr = crate::ai::skills::SkillManager::load_all(
                app_root.join("skills"),
                Some(&cid),
                app_root.join("characters"),
            );
            refresh_skills_ui(&sw, &mgr);
            sw.set_loaded_skills_summary(format!("重新扫描完成：已加载 {} 项技能", mgr.len()).into());
        }
    });

    let sw_weak_tmcp = settings_window.as_weak();
    let char_mgr_tmcp = Arc::clone(&char_manager);
    let config_tmcp = Arc::clone(&config);
    settings_window.on_toggle_mcp_server(move |name_slint, enabled| {
        let name = name_slint.to_string();
        if let Some(sw) = sw_weak_tmcp.upgrade() {
            let cid = sw.get_editing_char_id().to_string();
            let mut cm = char_mgr_tmcp.lock().unwrap();
            if let Some(char_cfg) = cm.get_character_mut(&cid) {
                if let Some(srv) = char_cfg.agent.mcp_servers.get_mut(&name) {
                    srv.enabled = enabled;
                }
                refresh_mcp_servers_ui(&sw, &char_cfg.agent.mcp_servers);
            }
            let mut cfg = config_tmcp.lock().unwrap();
            if let Some(srv) = cfg.agent.mcp_servers.get_mut(&name) {
                srv.enabled = enabled;
            }
            let _ = cfg.save_to_disk(crate::paths::config_path());
        }
    });

    let sw_weak_dmcp = settings_window.as_weak();
    let char_mgr_dmcp = Arc::clone(&char_manager);
    let config_dmcp = Arc::clone(&config);
    settings_window.on_delete_mcp_server(move |name_slint| {
        let name = name_slint.to_string();
        if let Some(sw) = sw_weak_dmcp.upgrade() {
            let cid = sw.get_editing_char_id().to_string();
            let mut cm = char_mgr_dmcp.lock().unwrap();
            if let Some(char_cfg) = cm.get_character_mut(&cid) {
                char_cfg.agent.mcp_servers.remove(&name);
                refresh_mcp_servers_ui(&sw, &char_cfg.agent.mcp_servers);
            }
            let mut cfg = config_dmcp.lock().unwrap();
            cfg.agent.mcp_servers.remove(&name);
            let _ = cfg.save_to_disk(crate::paths::config_path());
        }
    });

    let sw_weak_amcp = settings_window.as_weak();
    let char_mgr_amcp = Arc::clone(&char_manager);
    let config_amcp = Arc::clone(&config);
    settings_window.on_add_mcp_server(move |name_slint, transport_slint, cmd_slint, args_slint, url_slint| {
        let name = name_slint.to_string().trim().to_string();
        if name.is_empty() {
            return;
        }
        let transport = transport_slint.to_string().trim().to_string();
        let cmd_str = cmd_slint.to_string().trim().to_string();
        let args_str = args_slint.to_string();
        let url_str = url_slint.to_string().trim().to_string();

        let args_vec: Vec<String> = args_str
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();

        let srv_cfg = crate::config::McpServerConfig {
            enabled: true,
            transport: if transport == "sse" { "sse".to_string() } else { "stdio".to_string() },
            command: if cmd_str.is_empty() { None } else { Some(cmd_str) },
            args: args_vec,
            env: HashMap::new(),
            url: if url_str.is_empty() { None } else { Some(url_str) },
        };

        if let Some(sw) = sw_weak_amcp.upgrade() {
            let cid = sw.get_editing_char_id().to_string();
            let mut cm = char_mgr_amcp.lock().unwrap();
            if let Some(char_cfg) = cm.get_character_mut(&cid) {
                char_cfg.agent.mcp_servers.insert(name.clone(), srv_cfg.clone());
                refresh_mcp_servers_ui(&sw, &char_cfg.agent.mcp_servers);
            }
            let mut cfg = config_amcp.lock().unwrap();
            cfg.agent.mcp_servers.insert(name, srv_cfg);
            let _ = cfg.save_to_disk(crate::paths::config_path());
        }
    });

    let sw_weak_mcp = settings_window.as_weak();
    let char_mgr_mcp = Arc::clone(&char_manager);
    settings_window.on_refresh_mcp_clicked(move || {
        if let Some(sw) = sw_weak_mcp.upgrade() {
            let cid = sw.get_editing_char_id().to_string();
            let cm = char_mgr_mcp.lock().unwrap();
            let count = cm.get_character(&cid)
                .map(|c| c.agent.mcp_servers.len())
                .unwrap_or(0);
            sw.set_loaded_mcp_summary(format!("MCP 服务状态：已配置 {count} 个外部服务，连接就绪").into());
        }
    });
}
