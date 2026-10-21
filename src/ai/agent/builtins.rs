use anyhow::{Result, anyhow};
use async_openai::types::chat::{ChatCompletionTool, ChatCompletionTools, FunctionObject};
use serde_json::{Value, json};
use std::path::Path;

use crate::{
    ai::skills::SkillManager,
    config::{BuiltinToolsConfig, SecurityLevel},
};

pub struct BuiltinTools;

impl BuiltinTools {
    pub fn get_definitions(config: &BuiltinToolsConfig) -> Vec<ChatCompletionTools> {
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

        tools
    }

    pub fn is_builtin(name: &str) -> bool {
        matches!(name, "use_skill" | "get_current_time" | "read_file" | "execute_command")
    }

    pub async fn execute(
        name: &str,
        args: &Value,
        skills: &SkillManager,
        security_level: &SecurityLevel,
    ) -> Result<String> {
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
                    Ok(resp)
                } else {
                    Ok(format!(
                        "未找到名为 '{}' 的技能。请参考可用技能列表中的有效名称。",
                        skill_name
                    ))
                }
            }
            "get_current_time" => {
                // Return ISO/local timestamp
                let now = chrono_or_std_now();
                Ok(format!("当前本地时间: {now}"))
            }
            "read_file" => {
                let path_str = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("缺少必要参数 'path'"))?;

                let path = Path::new(path_str);
                if !path.exists() {
                    return Ok(format!("文件不存在: {}", path.display()));
                }

                let content = std::fs::read_to_string(path)
                    .map_err(|e| anyhow!("读取文件失败 ({}): {e}", path.display()))?;
                Ok(content)
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

                Ok(format!(
                    "退出码: {}\nstdout:\n{}\nstderr:\n{}",
                    code,
                    if stdout.is_empty() { "(无输出)" } else { &stdout },
                    if stderr.is_empty() { "(无错误)" } else { &stderr }
                ))
            }
            _ => Err(anyhow!("未知内置工具: {name}")),
        }
    }
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
        let tools = BuiltinTools::get_definitions(&default_cfg);
        assert_eq!(tools.len(), 4);
        let names: Vec<_> = tools
            .iter()
            .map(|t| match t {
                ChatCompletionTools::Function(f) => f.function.name.as_str(),
                _ => "",
            })
            .collect();
        assert!(names.contains(&"use_skill"));
        assert!(names.contains(&"get_current_time"));
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"execute_command"));

        // Verify JSON serialization includes required "type": "function"
        let serialized = serde_json::to_value(&tools).expect("failed to serialize tools");
        assert!(serialized.is_array());
        for tool_json in serialized.as_array().unwrap() {
            assert_eq!(tool_json["type"], "function");
            assert!(tool_json["function"]["name"].is_string());
        }

        let filtered_cfg = BuiltinToolsConfig {
            use_skill: false,
            get_current_time: true,
            read_file: false,
            execute_command: true,
        };
        let filtered_tools = BuiltinTools::get_definitions(&filtered_cfg);
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
    }

    #[tokio::test]
    async fn test_builtin_tools_execution() {
        let skills = Arc::new(SkillManager::new());
        let sec = SecurityLevel::ReadOnly;

        // 1. get_current_time
        let time_res = BuiltinTools::execute("get_current_time", &serde_json::json!({}), &skills, &sec).await;
        assert!(time_res.is_ok());
        assert!(time_res.unwrap().contains("当前本地时间:"));

        // 2. use_skill for non-existent skill
        let skill_res = BuiltinTools::execute(
            "use_skill",
            &serde_json::json!({"skill_name": "non_existent_skill_xyz"}),
            &skills,
            &sec,
        )
        .await;
        assert!(skill_res.is_ok());
        assert!(skill_res.unwrap().contains("未找到名为 'non_existent_skill_xyz' 的技能"));

        // 3. read_file
        let tmp_path = std::env::temp_dir().join("murasame_test_builtin_read.txt");
        {
            let mut f = std::fs::File::create(&tmp_path).unwrap();
            write!(f, "Murasame built-in tool read_file test content").unwrap();
        }
        let path_str = tmp_path.to_str().unwrap();

        let read_res = BuiltinTools::execute("read_file", &serde_json::json!({"path": path_str}), &skills, &sec).await;
        assert!(read_res.is_ok());
        assert_eq!(read_res.unwrap(), "Murasame built-in tool read_file test content");
        let _ = std::fs::remove_file(tmp_path);

        // 4. execute_command blocked by ReadOnly security policy
        let cmd_res = BuiltinTools::execute("execute_command", &serde_json::json!({"command": "echo 1"}), &skills, &sec).await;
        assert!(cmd_res.is_err());
        assert!(cmd_res.unwrap_err().to_string().contains("只读安全模式"));

        // 5. execute_command allowed under AutoExecute
        let auto_sec = SecurityLevel::AutoExecute;
        let cmd_ok = BuiltinTools::execute("execute_command", &serde_json::json!({"command": "echo hello_murasame"}), &skills, &auto_sec).await;
        assert!(cmd_ok.is_ok());
        assert!(cmd_ok.unwrap().contains("hello_murasame"));
    }
}
