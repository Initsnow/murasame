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
        if is_sentence_delimiter(ch) {
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
    matches!(ch, '。' | '！' | '？' | '!' | '?' | '\n' | '；' | ';')
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
}
