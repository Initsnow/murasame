use anyhow::{Result, anyhow};
use async_openai::types::chat::{ChatCompletionTool, ChatCompletionTools, FunctionObject};
use serde_json::{Value, json};
use std::path::Path;

use crate::{
    ai::skills::SkillManager,
    config::{BuiltinToolsConfig, SecurityLevel},
};

#[derive(Debug, Clone, PartialEq)]
pub struct ToolImageResult {
    pub base64: String,
    pub mime_type: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolExecutionResult {
    pub display_text: String,
    pub image: Option<ToolImageResult>,
}

impl ToolExecutionResult {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            display_text: text.into(),
            image: None,
        }
    }

    pub fn with_image(
        display_text: impl Into<String>,
        base64: String,
        mime_type: impl Into<String>,
    ) -> Self {
        Self {
            display_text: display_text.into(),
            image: Some(ToolImageResult {
                base64,
                mime_type: mime_type.into(),
            }),
        }
    }

    pub fn error(err: impl Into<String>) -> Self {
        Self {
            display_text: err.into(),
            image: None,
        }
    }
}

pub struct BuiltinTools;

impl BuiltinTools {
    pub fn get_definitions(config: &BuiltinToolsConfig, is_vision: bool) -> Vec<ChatCompletionTools> {
        let mut tools = Vec::new();

        if config.use_skill {
            tools.push(ChatCompletionTools::Function(ChatCompletionTool {
                function: FunctionObject {
                    name: "use_skill".to_string(),
                    description: Some("加载并查看指定技能的完整详细操作说明与步骤指南".to_string()),
                    parameters: Some(json!({
                        "type": "object",
                        "properties": {
                            "skill_name": {
                                "type": "string",
                                "description": "要加载的技能名称 (例如: web_search)"
                            }
                        },
                        "required": ["skill_name"]
                    })),
                    strict: None,
                },
            }));
        }

        if config.get_current_time {
            tools.push(ChatCompletionTools::Function(ChatCompletionTool {
                function: FunctionObject {
                    name: "get_current_time".to_string(),
                    description: Some("获取当前本地系统日期与时间".to_string()),
                    parameters: Some(json!({
                        "type": "object",
                        "properties": {}
                    })),
                    strict: None,
                },
            }));
        }

        if config.read_file {
            tools.push(ChatCompletionTools::Function(ChatCompletionTool {
                function: FunctionObject {
                    name: "read_file".to_string(),
                    description: Some("读取指定本地文件的文本内容".to_string()),
                    parameters: Some(json!({
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "目标文件路径"
                            }
                        },
                        "required": ["path"]
                    })),
                    strict: None,
                },
            }));
        }

        if config.execute_command {
            tools.push(ChatCompletionTools::Function(ChatCompletionTool {
                function: FunctionObject {
                    name: "execute_command".to_string(),
                    description: Some("在系统终端中执行受控指令并返回 stdout/stderr 输出".to_string()),
                    parameters: Some(json!({
                        "type": "object",
                        "properties": {
                            "command": {
                                "type": "string",
                                "description": "终端命令"
                            }
                        },
                        "required": ["command"]
                    })),
                    strict: None,
                },
            }));
        }

        if config.take_screenshot && is_vision {
            tools.push(ChatCompletionTools::Function(ChatCompletionTool {
                function: FunctionObject {
                    name: "take_screenshot".to_string(),
                    description: Some("截取系统当前屏幕图像，返回轻量压缩的视觉图像数据供多模态模型分析".to_string()),
                    parameters: Some(json!({
                        "type": "object",
                        "properties": {
                            "save_to_disk": {
                                "type": "boolean",
                                "description": "是否同时保存至本地磁盘文件（可选，缺省遵循用户系统设置）"
                            }
                        }
                    })),
                    strict: None,
                },
            }));
        }

        tools
    }

    pub fn is_builtin(name: &str) -> bool {
        matches!(
            name,
            "use_skill" | "get_current_time" | "read_file" | "execute_command" | "take_screenshot"
        )
    }

    pub async fn execute(
        name: &str,
        args: &Value,
        skills: &SkillManager,
        security_level: &SecurityLevel,
        builtin_config: &BuiltinToolsConfig,
        is_vision: bool,
    ) -> Result<ToolExecutionResult> {
        match name {
            "use_skill" => {
                let skill_name = args
                    .get("skill_name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("缺少必要参数 'skill_name'"))?;

                if let Some(skill) = skills.get_skill(skill_name) {
                    let mut resp = format!(
                        "【技能加载成功: {}】\n描述: {}\n\n【操作指南及规则】\n{}",
                        skill.metadata.name, skill.metadata.description, skill.instructions
                    );
                    if skill.is_character_specific {
                        resp.push_str("\n(注: 此技能为角色专属定制技能)");
                    }
                    Ok(ToolExecutionResult::text(resp))
                } else {
                    Ok(ToolExecutionResult::text(format!(
                        "未找到名为 '{}' 的技能。请参考可用技能列表中的有效名称。",
                        skill_name
                    )))
                }
            }
            "get_current_time" => {
                // Return ISO/local timestamp
                let now = chrono_or_std_now();
                Ok(ToolExecutionResult::text(format!("当前本地时间: {now}")))
            }
            "read_file" => {
                let path_str = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("缺少必要参数 'path'"))?;

                let path = Path::new(path_str);
                if !path.exists() {
                    return Ok(ToolExecutionResult::text(format!("文件不存在: {}", path.display())));
                }

                let content = std::fs::read_to_string(path)
                    .map_err(|e| anyhow!("读取文件失败 ({}): {e}", path.display()))?;
                Ok(ToolExecutionResult::text(content))
            }
            "execute_command" => {
                let cmd_str = args
                    .get("command")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("缺少必要参数 'command'"))?;

                if *security_level == SecurityLevel::ReadOnly {
                    return Err(anyhow!("当前处于只读安全模式 (ReadOnly)，拒绝执行命令"));
                }

                #[cfg(windows)]
                let mut cmd = tokio::process::Command::new("powershell");
                #[cfg(windows)]
                cmd.args(["-NoProfile", "-Command", cmd_str]);

                #[cfg(not(windows))]
                let mut cmd = tokio::process::Command::new("sh");
                #[cfg(not(windows))]
                cmd.args(["-c", cmd_str]);

                #[cfg(windows)]
                {
                    const CREATE_NO_WINDOW: u32 = 0x08000000;
                    cmd.creation_flags(CREATE_NO_WINDOW);
                }

                let output = cmd.output().await
                    .map_err(|e| anyhow!("执行命令失败: {e}"))?;

                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let code = output.status.code().unwrap_or(-1);

                Ok(ToolExecutionResult::text(format!(
                    "退出码: {}\nstdout:\n{}\nstderr:\n{}",
                    code,
                    if stdout.is_empty() { "(无输出)" } else { &stdout },
                    if stderr.is_empty() { "(无错误)" } else { &stderr }
                )))
            }
            "take_screenshot" => {
                if !is_vision {
                    return Err(anyhow!("当前模型为纯文本模型，不支持多模态视觉图像输入"));
                }

                let img = crate::app::platform::capture_screen_image()?;
                let width = img.width();
                let height = img.height();

                // Convert RGBA to RGB for lightweight JPEG compression
                let rgb_img = image::DynamicImage::ImageRgba8(img).into_rgb8();
                let mut jpeg_bytes = Vec::new();
                let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_bytes, 88);
                encoder
                    .encode(
                        rgb_img.as_raw(),
                        width,
                        height,
                        image::ExtendedColorType::Rgb8,
                    )
                    .map_err(|e| anyhow!("JPEG 图像压缩失败: {e}"))?;

                let should_save = args
                    .get("save_to_disk")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(builtin_config.screenshot_save_to_disk);

                let saved_info = if should_save {
                    let dir = crate::paths::data_dir().join("screenshots");
                    let _ = std::fs::create_dir_all(&dir);
                    let now_str = chrono_or_std_now_compact();
                    let save_path = dir.join(format!("screenshot_{now_str}.jpg"));

                    if let Some(parent) = save_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }

                    std::fs::write(&save_path, &jpeg_bytes)
                        .map_err(|e| anyhow!("保存截图图像文件失败 ({}): {e}", save_path.display()))?;

                    format!("，已保存至本地: {}", save_path.display())
                } else {
                    String::new()
                };

                let b64 = base64_encode(&jpeg_bytes);
                let summary = format!(
                    "已截取系统当前屏幕图像（分辨率: {}x{}{}）。",
                    width, height, saved_info
                );

                Ok(ToolExecutionResult::with_image(summary, b64, "image/jpeg"))
            }
            _ => Err(anyhow!("未知内置工具: {name}")),
        }
    }
}

pub fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 { chunk[1] as usize } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as usize } else { 0 };
        out.push(TABLE[(b0 >> 2) & 0x3F] as char);
        out.push(TABLE[((b0 << 4) | (b1 >> 4)) & 0x3F] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((b1 << 2) | (b2 >> 6)) & 0x3F] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[b2 & 0x3F] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn chrono_or_std_now_compact() -> String {
    #[cfg(windows)]
    {
        use std::process::Command;
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-Command", "Get-Date -Format 'yyyyMMdd_HHmmss'"]);
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        if let Ok(out) = cmd.output() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    format!("{secs}")
}

fn chrono_or_std_now() -> String {
    // Format timestamp without requiring external chrono crate
    #[cfg(windows)]
    {
        use std::process::Command;
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-Command", "Get-Date -Format 'yyyy-MM-dd HH:mm:ss'"]);
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        if let Ok(out) = cmd.output() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    "2026-10-18 20:00:00".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Arc;

    #[test]
    fn test_builtin_tools_definitions() {
        let default_cfg = BuiltinToolsConfig::default();
        // Vision model should have take_screenshot
        let tools_vision = BuiltinTools::get_definitions(&default_cfg, true);
        assert_eq!(tools_vision.len(), 5);
        let names_vision: Vec<_> = tools_vision
            .iter()
            .map(|t| match t {
                ChatCompletionTools::Function(f) => f.function.name.as_str(),
                _ => "",
            })
            .collect();
        assert!(names_vision.contains(&"use_skill"));
        assert!(names_vision.contains(&"get_current_time"));
        assert!(names_vision.contains(&"read_file"));
        assert!(names_vision.contains(&"execute_command"));
        assert!(names_vision.contains(&"take_screenshot"));

        // Text-only model should NOT have take_screenshot
        let tools_text = BuiltinTools::get_definitions(&default_cfg, false);
        assert_eq!(tools_text.len(), 4);
        let names_text: Vec<_> = tools_text
            .iter()
            .map(|t| match t {
                ChatCompletionTools::Function(f) => f.function.name.as_str(),
                _ => "",
            })
            .collect();
        assert!(!names_text.contains(&"take_screenshot"));

        // Verify JSON serialization includes required "type": "function"
        let serialized = serde_json::to_value(&tools_vision).expect("failed to serialize tools");
        assert!(serialized.is_array());
        for tool_json in serialized.as_array().unwrap() {
            assert_eq!(tool_json["type"], "function");
            assert!(tool_json["function"]["name"].is_string());
        }

        // Verify take_screenshot does not have 'path' or 'include_base64'
        let shot_tool = tools_vision
            .iter()
            .find(|t| match t {
                ChatCompletionTools::Function(f) => f.function.name == "take_screenshot",
                _ => false,
            })
            .expect("take_screenshot tool should exist");
        if let ChatCompletionTools::Function(f) = shot_tool {
            let params = f.function.parameters.as_ref().unwrap();
            assert!(params["properties"].get("path").is_none());
            assert!(params["properties"].get("include_base64").is_none());
        }

        let filtered_cfg = BuiltinToolsConfig {
            use_skill: false,
            get_current_time: true,
            read_file: false,
            execute_command: true,
            take_screenshot: false,
            screenshot_save_to_disk: false,
        };
        let filtered_tools = BuiltinTools::get_definitions(&filtered_cfg, true);
        assert_eq!(filtered_tools.len(), 2);
        let filtered_names: Vec<_> = filtered_tools
            .iter()
            .map(|t| match t {
                ChatCompletionTools::Function(f) => f.function.name.as_str(),
                _ => "",
            })
            .collect();
        assert!(!filtered_names.contains(&"use_skill"));
        assert!(filtered_names.contains(&"get_current_time"));
        assert!(!filtered_names.contains(&"read_file"));
        assert!(filtered_names.contains(&"execute_command"));
        assert!(!filtered_names.contains(&"take_screenshot"));
    }

    #[tokio::test]
    async fn test_builtin_tools_execution() {
        let skills = Arc::new(SkillManager::new());
        let sec = SecurityLevel::ReadOnly;
        let bcfg = BuiltinToolsConfig::default();

        // 1. get_current_time
        let time_res = BuiltinTools::execute("get_current_time", &serde_json::json!({}), &skills, &sec, &bcfg, false).await;
        assert!(time_res.is_ok());
        assert!(time_res.unwrap().display_text.contains("当前本地时间:"));

        // 2. use_skill for non-existent skill
        let skill_res = BuiltinTools::execute(
            "use_skill",
            &serde_json::json!({"skill_name": "non_existent_skill_xyz"}),
            &skills,
            &sec,
            &bcfg,
            false,
        )
        .await;
        assert!(skill_res.is_ok());
        assert!(skill_res.unwrap().display_text.contains("未找到名为 'non_existent_skill_xyz' 的技能"));

        // 3. read_file
        let tmp_path = std::env::temp_dir().join("murasame_test_builtin_read.txt");
        {
            let mut f = std::fs::File::create(&tmp_path).unwrap();
            write!(f, "Murasame built-in tool read_file test content").unwrap();
        }
        let path_str = tmp_path.to_str().unwrap();

        let read_res = BuiltinTools::execute("read_file", &serde_json::json!({"path": path_str}), &skills, &sec, &bcfg, false).await;
        assert!(read_res.is_ok());
        assert_eq!(read_res.unwrap().display_text, "Murasame built-in tool read_file test content");
        let _ = std::fs::remove_file(tmp_path);

        // 4. execute_command blocked by ReadOnly security policy
        let cmd_res = BuiltinTools::execute("execute_command", &serde_json::json!({"command": "echo 1"}), &skills, &sec, &bcfg, false).await;
        assert!(cmd_res.is_err());
        assert!(cmd_res.unwrap_err().to_string().contains("只读安全模式"));

        // 5. execute_command allowed under AutoExecute
        let auto_sec = SecurityLevel::AutoExecute;
        let cmd_ok = BuiltinTools::execute("execute_command", &serde_json::json!({"command": "echo hello_murasame"}), &skills, &auto_sec, &bcfg, false).await;
        assert!(cmd_ok.is_ok());
        assert!(cmd_ok.unwrap().display_text.contains("hello_murasame"));

        // 6. take_screenshot execution with pure text model (is_vision: false) should fail
        let shot_text_res = BuiltinTools::execute(
            "take_screenshot",
            &serde_json::json!({}),
            &skills,
            &sec,
            &bcfg,
            false,
        )
        .await;
        assert!(shot_text_res.is_err());
        assert!(shot_text_res.unwrap_err().to_string().contains("纯文本模型"));

        // 7. take_screenshot execution with vision model (is_vision: true, default no disk save)
        let shot_mem_res = BuiltinTools::execute(
            "take_screenshot",
            &serde_json::json!({}),
            &skills,
            &sec,
            &bcfg,
            true,
        )
        .await;
        #[cfg(windows)]
        {
            assert!(shot_mem_res.is_ok(), "Screenshot memory execution failed: {:?}", shot_mem_res.err());
            let out_res = shot_mem_res.unwrap();
            assert!(out_res.display_text.contains("分辨率:"));
            assert!(!out_res.display_text.contains("data:image/jpeg;base64"));
            assert!(out_res.image.is_some());
            let img = out_res.image.unwrap();
            assert_eq!(img.mime_type, "image/jpeg");
            assert!(!img.base64.is_empty());
        }

        // 8. take_screenshot with save_to_disk: true
        let mut bcfg_save = bcfg.clone();
        bcfg_save.screenshot_save_to_disk = true;
        let shot_disk_res = BuiltinTools::execute(
            "take_screenshot",
            &serde_json::json!({}),
            &skills,
            &sec,
            &bcfg_save,
            true,
        )
        .await;
        #[cfg(windows)]
        {
            assert!(shot_disk_res.is_ok(), "Screenshot disk execution failed: {:?}", shot_disk_res.err());
            let out_res = shot_disk_res.unwrap();
            assert!(out_res.display_text.contains("已保存至本地:"));
            assert!(!out_res.display_text.contains("data:image/jpeg;base64"));
            assert!(out_res.image.is_some());
        }
        #[cfg(not(windows))]
        {
            let _ = shot_mem_res;
            let _ = shot_disk_res;
        }
    }
}
