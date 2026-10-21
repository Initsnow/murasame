use crate::model::CueRegistry;

#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// Valid whitelisted Live2D cue (e.g. "smile", "greet")
    Cue(String),
    /// Clean text chunk without intercepted cues (for subtitles)
    Text(String),
    /// Completed sentence boundary (for TTS streaming playback)
    Sentence(String),
}

pub struct StreamCueParser {
    cues: CueRegistry,
    bracket_buffer: Option<String>,
    sentence_buffer: String,
    sentence_char_count: usize,
}

impl StreamCueParser {
    pub fn new(cues: CueRegistry) -> Self {
        Self {
            cues,
            bracket_buffer: None,
            sentence_buffer: String::new(),
            sentence_char_count: 0,
        }
    }

    pub fn process_token(&mut self, token: &str) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        let mut normal_text = String::new();

        for ch in token.chars() {
            if let Some(mut buf) = self.bracket_buffer.take() {
                buf.push(ch);
                if ch == ']' {
                    let candidate = &buf[1..buf.len() - 1];
                    if self.cues.is_valid_cue(candidate) {
                        events.push(StreamEvent::Cue(candidate.to_string()));
                    } else {
                        // Not in whitelist, emit bracket text as normal text
                        events.push(StreamEvent::Text(buf.clone()));
                        self.push_sentence_char_sequence(&buf, &mut events);
                    }
                } else if buf.len() > 64 || ch == '[' || ch == '\n' {
                    // Exceeded maximum tag length or nested bracket, flush buffer
                    events.push(StreamEvent::Text(buf.clone()));
                    self.push_sentence_char_sequence(&buf, &mut events);
                    if ch == '[' {
                        self.bracket_buffer = Some("[".to_string());
                    }
                } else {
                    self.bracket_buffer = Some(buf);
                }
            } else if ch == '[' {
                if !normal_text.is_empty() {
                    events.push(StreamEvent::Text(std::mem::take(&mut normal_text)));
                }
                self.bracket_buffer = Some("[".to_string());
            } else {
                normal_text.push(ch);
                self.push_sentence_char(ch, &mut events);
            }
        }

        if !normal_text.is_empty() {
            events.push(StreamEvent::Text(normal_text));
        }

        events
    }

    fn push_sentence_char(&mut self, ch: char, events: &mut Vec<StreamEvent>) {
        self.sentence_buffer.push(ch);
        self.sentence_char_count += 1;
        let char_count = self.sentence_char_count;
        let should_split = if is_sentence_delimiter(ch) {
            true
        } else if char_count >= 80 && (ch == '，' || ch == ',' || ch == ' ') {
            true
        } else {
            char_count >= 120
        };

        if should_split {
            let sentence = self.sentence_buffer.trim().to_string();
            if !sentence.is_empty() {
                events.push(StreamEvent::Sentence(sentence));
            }
            self.sentence_buffer.clear();
            self.sentence_char_count = 0;
        }
    }

    fn push_sentence_char_sequence(&mut self, seq: &str, events: &mut Vec<StreamEvent>) {
        for ch in seq.chars() {
            self.push_sentence_char(ch, events);
        }
    }

    pub fn finish(&mut self) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        if let Some(buf) = self.bracket_buffer.take() {
            events.push(StreamEvent::Text(buf.clone()));
            self.push_sentence_char_sequence(&buf, &mut events);
        }
        let sentence = self.sentence_buffer.trim().to_string();
        if !sentence.is_empty() {
            events.push(StreamEvent::Sentence(sentence));
        }
        self.sentence_buffer.clear();
        self.sentence_char_count = 0;
        events
    }
}

fn is_sentence_delimiter(ch: char) -> bool {
    matches!(ch, '。' | '！' | '？' | '!' | '?' | '\n' | '；' | ';' | '…')
}

pub fn is_cue_tag(tag: &str, cues: Option<&CueRegistry>) -> bool {
    if let Some(c) = cues {
        c.is_valid_cue(tag)
    } else {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedMessageParts {
    pub has_thought: bool,
    pub thought: String,
    pub has_tools: bool,
    pub tools_summary: String,
    pub tools_detail: String,
    pub clean_text: String,
}

/// Parses message content into thinking, tool executions, and final conversational text
pub fn extract_message_parts(raw: &str) -> ParsedMessageParts {
    let re_thought =
        regex::Regex::new(r#"(?s)<(?:thought|think)\b[^>]*>(.*?)</(?:thought|think)>"#).unwrap();
    let re_tool_call = regex::Regex::new(
        r#"(?s)<client_tool_call\s+name=["']?([^"'>\s]+)["']?\s*>(.*?)</client_tool_call>"#,
    )
    .unwrap();
    let re_tool_result = regex::Regex::new(
        r#"(?s)<client_tool_result\s+name=["']?([^"'>\s]+)["']?\s*>(.*?)</client_tool_result>"#,
    )
    .unwrap();
    let re_context =
        regex::Regex::new(r#"(?s)<client_context\b[^>]*>.*?</client_context>"#).unwrap();

    // 1. Extract thought
    let mut thoughts = Vec::new();
    for cap in re_thought.captures_iter(raw) {
        let t = cap[1].trim();
        if !t.is_empty() {
            thoughts.push(t.to_string());
        }
    }
    let thought_str = thoughts.join("\n\n");
    let has_thought = !thought_str.is_empty();

    // 2. Extract tool calls & results
    let mut tool_calls = Vec::new();
    for cap in re_tool_call.captures_iter(raw) {
        let name = cap[1].trim().to_string();
        let args = cap[2].trim().to_string();
        tool_calls.push((name, args));
    }

    let mut tool_results = Vec::new();
    for cap in re_tool_result.captures_iter(raw) {
        let res = cap[2].trim().to_string();
        tool_results.push(res);
    }

    let has_tools = !tool_calls.is_empty();
    let tools_summary = if tool_calls.len() == 1 {
        format!("已执行工具: {}", tool_calls[0].0)
    } else if tool_calls.len() > 1 {
        format!("已执行 {} 项工具调用", tool_calls.len())
    } else {
        String::new()
    };

    let mut details = Vec::new();
    for (i, (name, args)) in tool_calls.iter().enumerate() {
        let res = tool_results.get(i).map(|r| r.as_str()).unwrap_or("");
        details.push(format!(
            "工具: {}\n参数: {}\n结果:\n{}",
            name,
            if args.is_empty() { "{}" } else { args },
            if res.is_empty() { "(无输出)" } else { res }
        ));
    }
    let tools_detail = details.join("\n--------------------\n");

    // 3. Clean text (remove XML tags)
    let step1 = re_thought.replace_all(raw, "");
    let step2 = re_tool_call.replace_all(&step1, "");
    let step3 = re_tool_result.replace_all(&step2, "");
    let step4 = re_context.replace_all(&step3, "");
    let clean_text = step4.trim().to_string();

    ParsedMessageParts {
        has_thought,
        thought: thought_str,
        has_tools,
        tools_summary,
        tools_detail,
        clean_text,
    }
}

/// Defensively strips any XML tool call, tool result, context, or thought tags
pub fn strip_tool_xml(text: &str) -> String {
    extract_message_parts(text).clean_text
}

pub fn strip_cues(text: &str, cues: Option<&CueRegistry>) -> String {
    let clean_xml = strip_tool_xml(text);
    let mut result = String::with_capacity(clean_xml.len());
    let mut chars = clean_xml.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '[' {
            let mut tag_content = String::new();
            let mut is_tag = false;
            let mut buffer = String::from("[");

            while let Some(&next_ch) = chars.peek() {
                if next_ch == ']' {
                    chars.next();
                    buffer.push(']');
                    is_tag = true;
                    break;
                } else if next_ch == '\n' || next_ch == '[' || buffer.len() > 64 {
                    break;
                } else {
                    tag_content.push(next_ch);
                    buffer.push(next_ch);
                    chars.next();
                }
            }

            if is_tag {
                // If followed by '(' -> markdown link e.g. [text](url), keep it!
                if chars.peek() == Some(&'(') {
                    result.push_str(&buffer);
                    continue;
                }

                let trimmed = tag_content.trim();
                let is_cue = is_cue_tag(trimmed, cues);
                if is_cue {
                    continue;
                } else {
                    result.push_str(&buffer);
                }
            } else {
                result.push_str(&buffer);
            }
        } else {
            result.push(ch);
        }
    }

    let mut cleaned_lines = Vec::new();
    for line in result.lines() {
        let mut line_buf = String::with_capacity(line.len());
        let mut last_was_space = false;
        for c in line.trim().chars() {
            if c == ' ' {
                if !last_was_space {
                    line_buf.push(c);
                    last_was_space = true;
                }
            } else {
                line_buf.push(c);
                last_was_space = false;
            }
        }
        cleaned_lines.push(line_buf);
    }

    cleaned_lines.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_cue_interception_and_whitelist() {
        let mut registry = CueRegistry::new();
        registry.register_expression("smile", PathBuf::from("exp1.exp3"));
        registry.register_motion("greet", PathBuf::from("motion02.motion3.json"));

        let mut parser = StreamCueParser::new(registry);

        let events1 = parser.process_token("[smi");
        assert!(events1.is_empty());

        let events2 = parser.process_token("le]早上好！[greet]");
        assert!(events2.contains(&StreamEvent::Cue("smile".to_string())));
        assert!(events2.contains(&StreamEvent::Cue("greet".to_string())));
        assert!(events2.contains(&StreamEvent::Sentence("早上好！".to_string())));

        // Test non-whitelist tags like [1]
        let events3 = parser.process_token("参考[1]测试");
        let finish_events = parser.finish();

        let all_text = events3
            .into_iter()
            .chain(finish_events)
            .filter_map(|e| match e {
                StreamEvent::Text(t) => Some(t),
                _ => None,
            })
            .collect::<String>();

        assert_eq!(all_text, "参考[1]测试");
    }

    #[test]
    fn test_strip_cues_function() {
        let mut registry = CueRegistry::new();
        registry.register_expression("smile", PathBuf::from("exp1.exp3"));
        registry.register_motion("greet", PathBuf::from("motion02.motion3.json"));

        let input = "[smile] 早上好！[greet] 今天想做什么呢？[unregistered]";
        let stripped = strip_cues(input, Some(&registry));
        assert_eq!(stripped, "早上好！ 今天想做什么呢？[unregistered]");

        // Markdown link preservation
        let md_input = "请参考 [官方文档](https://example.com) 进行配置 [smile]";
        let md_stripped = strip_cues(md_input, Some(&registry));
        assert_eq!(
            md_stripped,
            "请参考 [官方文档](https://example.com) 进行配置"
        );
    }

    #[test]
    fn test_strip_tool_xml() {
        let text = "你好！<thought>思考中...</thought><client_tool_call name=\"read_file\">{\"path\":\"test.txt\"}</client_tool_call>文件已读取。<client_tool_result name=\"read_file\">内容</client_tool_result>祝你愉快！[smile]";
        let cleaned = strip_cues(text, None);
        assert_eq!(cleaned, "你好！文件已读取。祝你愉快！[smile]");
    }

    #[test]
    fn test_extract_message_parts() {
        let text = "<thought>\n需要先读取文件并执行指令\n</thought>\n<client_tool_call name=\"read_file\">\n{\"path\": \"config.toml\"}\n</client_tool_call>\n<client_tool_result name=\"read_file\">\n[执行成功]\nactive_character = \"murasame\"\n</client_tool_result>\n已经帮你看过文件了！[smile]";
        let parts = extract_message_parts(text);
        assert!(parts.has_thought);
        assert_eq!(parts.thought, "需要先读取文件并执行指令");
        assert!(parts.has_tools);
        assert!(parts.tools_summary.contains("read_file"));
        assert!(parts.tools_detail.contains("active_character = \"murasame\""));
        assert_eq!(parts.clean_text, "已经帮你看过文件了！[smile]");
    }
}
