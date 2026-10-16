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
}

impl StreamCueParser {
    pub fn new(cues: CueRegistry) -> Self {
        Self {
            cues,
            bracket_buffer: None,
            sentence_buffer: String::new(),
        }
    }

    pub fn process_token(&mut self, token: &str) -> Vec<StreamEvent> {
        let mut events = Vec::new();

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
                self.bracket_buffer = Some("[".to_string());
            } else {
                let s = ch.to_string();
                events.push(StreamEvent::Text(s));
                self.push_sentence_char(ch, &mut events);
            }
        }

        events
    }

    fn push_sentence_char(&mut self, ch: char, events: &mut Vec<StreamEvent>) {
        self.sentence_buffer.push(ch);
        let char_count = self.sentence_buffer.chars().count();
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

pub fn strip_cues(text: &str, cues: Option<&CueRegistry>) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

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
}
