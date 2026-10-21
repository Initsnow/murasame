use super::model::Skill;
use crate::ai::mcp::protocol::McpTool;
use crate::config::BuiltinToolsConfig;

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Renders a compact catalog of available skills for progressive disclosure in system prompt
pub fn render_skills_prompt(skills: &[&Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    out.push_str("\n\n[可用技能库 (Skills)]\n");
    out.push_str("你可以按需激活并调用以下技能。若需查看某项技能的详细工作流与指导，请调用 `use_skill` 工具：\n");
    out.push_str("<available_skills>\n");

    for skill in skills {
        out.push_str(&format!(
            "  <skill name=\"{}\" description=\"{}\"/>\n",
            skill.metadata.name,
            xml_escape(&skill.metadata.description)
        ));
    }

    out.push_str("</available_skills>\n");
    out.push_str("当用户明确要求某技能或任务流程复杂时，务必先调用 `use_skill` 获取规范步骤后再执行。\n");
    out
}

/// Renders the complete <client_context> XML declaration for Hermes protocol
pub fn render_client_context(
    skills: &[&Skill],
    builtin_tools: &BuiltinToolsConfig,
    mcp_tools: &[(&str, &McpTool)],
    is_vision: bool,
) -> String {
    let mut out = String::new();
    out.push_str("<client_context platform=\"murasame-desktop\">\n");

    // 1. available_skills
    out.push_str("  <available_skills>\n");
    for skill in skills {
        out.push_str(&format!(
            "    <skill name=\"{}\" description=\"{}\" />\n",
            xml_escape(&skill.metadata.name),
            xml_escape(&skill.metadata.description)
        ));
    }
    out.push_str("  </available_skills>\n\n");

    // 2. client_tools
    out.push_str("  <client_tools>\n");
    if builtin_tools.get_current_time {
        out.push_str("    <tool name=\"get_current_time\" description=\"获取客户端当前精确时间\">\n");
        out.push_str("      <parameters>{}</parameters>\n");
        out.push_str("    </tool>\n");
    }
    if builtin_tools.read_file {
        out.push_str("    <tool name=\"read_file\" description=\"读取客户端本地文件内容\">\n");
        out.push_str("      <parameters>{\"type\":\"object\",\"properties\":{\"path\":{\"type\":\"string\",\"description\":\"文件绝对或相对路径\"}},\"required\":[\"path\"]}</parameters>\n");
        out.push_str("    </tool>\n");
    }
    if builtin_tools.execute_command {
        out.push_str("    <tool name=\"execute_command\" description=\"在客户端执行系统命令行指令\">\n");
        out.push_str("      <parameters>{\"type\":\"object\",\"properties\":{\"command\":{\"type\":\"string\",\"description\":\"命令字符串\"}},\"required\":[\"command\"]}</parameters>\n");
        out.push_str("    </tool>\n");
    }
    if builtin_tools.use_skill {
        out.push_str("    <tool name=\"use_skill\" description=\"展开并加载指定本地技能的详细操作指导\">\n");
        out.push_str("      <parameters>{\"type\":\"object\",\"properties\":{\"skill_name\":{\"type\":\"string\",\"description\":\"技能名称\"}},\"required\":[\"skill_name\"]}</parameters>\n");
        out.push_str("    </tool>\n");
    }
    if builtin_tools.take_screenshot && is_vision {
        out.push_str("    <tool name=\"take_screenshot\" description=\"截取系统当前屏幕图像，返回轻量压缩的视觉图像数据供多模态模型分析\">\n");
        out.push_str("      <parameters>{\"type\":\"object\",\"properties\":{\"save_to_disk\":{\"type\":\"boolean\",\"description\":\"是否同时保存至本地磁盘文件（可选，缺省遵循用户系统设置）\"}}}</parameters>\n");
        out.push_str("    </tool>\n");
    }

    // MCP tools
    for (_server_name, tool) in mcp_tools {
        let desc = tool.description.as_deref().unwrap_or("");
        let params_json =
            serde_json::to_string(&tool.input_schema).unwrap_or_else(|_| "{}".to_string());
        out.push_str(&format!(
            "    <tool name=\"{}\" description=\"{}\">\n      <parameters>{}</parameters>\n    </tool>\n",
            xml_escape(&tool.name),
            xml_escape(desc),
            params_json
        ));
    }
    out.push_str("  </client_tools>\n\n");

    // 3. protocol_rules
    out.push_str("  <protocol_rules>\n");
    out.push_str("    当你需要调用客户端工具时，必须且仅能输出如下格式的 XML 标签：\n");
    out.push_str("    <client_tool_call name=\"工具名称\">\n");
    out.push_str("    {\"参数名\": \"参数值\"}\n");
    out.push_str("    </client_tool_call>\n");
    out.push_str("    客户端在本地执行完成后，会将结果以 <client_tool_result> 形式回传给你，之后你需结合人设给出最终自然语言回答。\n");
    out.push_str("  </protocol_rules>\n");
    out.push_str("</client_context>");

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::skills::model::SkillMetadata;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn test_render_skills_prompt() {
        let skill1 = Skill {
            metadata: SkillMetadata {
                name: "search".to_string(),
                description: "Search web".to_string(),
                version: None,
                author: None,
            },
            instructions: "step 1".to_string(),
            dir_path: PathBuf::new(),
            file_path: PathBuf::new(),
            is_character_specific: false,
        };

        let rendered = render_skills_prompt(&[&skill1]);
        assert!(rendered.contains("<available_skills>"));
        assert!(rendered.contains("name=\"search\""));
        assert!(rendered.contains("description=\"Search web\""));
        assert!(rendered.contains("use_skill"));
    }

    #[test]
    fn test_render_client_context() {
        let skill1 = Skill {
            metadata: SkillMetadata {
                name: "system_info".to_string(),
                description: "系统运行状态与硬件信息查看".to_string(),
                version: None,
                author: None,
            },
            instructions: "info".to_string(),
            dir_path: PathBuf::new(),
            file_path: PathBuf::new(),
            is_character_specific: false,
        };

        let builtin_cfg = BuiltinToolsConfig {
            use_skill: true,
            get_current_time: true,
            read_file: true,
            execute_command: true,
            take_screenshot: true,
            screenshot_save_to_disk: false,
        };

        let mcp_tool = McpTool {
            name: "sqlite_query".to_string(),
            description: Some("执行 SQL 查询".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sql": { "type": "string" }
                },
                "required": ["sql"]
            }),
        };

        let mcp_tools = [("sqlite", &mcp_tool)];
        let context_xml = render_client_context(&[&skill1], &builtin_cfg, &mcp_tools, true);

        assert!(context_xml.contains("<client_context platform=\"murasame-desktop\">"));
        assert!(context_xml.contains("<available_skills>"));
        assert!(context_xml.contains("<skill name=\"system_info\" description=\"系统运行状态与硬件信息查看\" />"));
        assert!(context_xml.contains("<client_tools>"));
        assert!(context_xml.contains("<tool name=\"get_current_time\" description=\"获取客户端当前精确时间\">"));
        assert!(context_xml.contains("<tool name=\"read_file\" description=\"读取客户端本地文件内容\">"));
        assert!(context_xml.contains("<tool name=\"execute_command\" description=\"在客户端执行系统命令行指令\">"));
        assert!(context_xml.contains("<tool name=\"use_skill\" description=\"展开并加载指定本地技能的详细操作指导\">"));
        assert!(context_xml.contains("<tool name=\"take_screenshot\" description=\"截取系统当前屏幕图像，返回轻量压缩的视觉图像数据供多模态模型分析\">"));
        let shot_section = context_xml
            .lines()
            .skip_while(|l| !l.contains("<tool name=\"take_screenshot\""))
            .take(3)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!shot_section.contains("include_base64"));
        assert!(!shot_section.contains("\"path\""));
        assert!(context_xml.contains("<tool name=\"sqlite_query\" description=\"执行 SQL 查询\">"));
        assert!(context_xml.contains("<protocol_rules>"));
        assert!(context_xml.contains("<client_tool_call name=\"工具名称\">"));
        assert!(context_xml.contains("</client_context>"));

        // Text-only model: take_screenshot should NOT be present
        let text_context_xml = render_client_context(&[&skill1], &builtin_cfg, &mcp_tools, false);
        assert!(!text_context_xml.contains("<tool name=\"take_screenshot\""));
    }
}
