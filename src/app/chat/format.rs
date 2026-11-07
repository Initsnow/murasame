use crate::app::SubtitleWindow;

pub fn prepare_markdown_for_slint(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_code_block = false;

    for line in text.lines() {
        let trimmed = line.trim();

        // 1. Handle code blocks ```...
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }

        if in_code_block {
            if !trimmed.is_empty() {
                if trimmed.contains('`') {
                    result.push_str(line);
                } else {
                    result.push('`');
                    result.push_str(trimmed);
                    result.push('`');
                }
            }
            result.push('\n');
            continue;
        }

        // 2. Handle headings # Heading -> **Heading**
        if let Some(heading_content) = trimmed.strip_prefix('#') {
            let heading_text = heading_content.trim_start_matches('#').trim();
            if !heading_text.is_empty() {
                result.push_str("**");
                result.push_str(heading_text);
                result.push_str("**  \n");
                continue;
            }
        }

        // 3. Handle blockquotes > Quote -> ▍ Quote
        if let Some(quote_content) = trimmed.strip_prefix('>') {
            result.push_str("▍ ");
            result.push_str(quote_content.trim());
            result.push_str("  \n");
            continue;
        }

        if !trimmed.is_empty() {
            result.push_str(line);
            if !line.ends_with("  ") {
                result.push_str("  ");
            }
            result.push('\n');
        } else {
            result.push('\n');
        }
    }

    if !text.ends_with('\n') && result.ends_with('\n') {
        result.pop();
        if result.ends_with("  ") {
            result.pop();
            result.pop();
        }
    }

    // 4. Sanitize unsupported HTML tags (keep only <u> and <font>)
    sanitize_html_tags(&result)
}

fn sanitize_html_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' {
            let mut tag = String::new();
            let mut closed = false;
            while let Some(&c) = chars.peek() {
                chars.next();
                if c == '>' {
                    closed = true;
                    break;
                }
                tag.push(c);
                if tag.len() > 128 {
                    break;
                }
            }
            if closed {
                let trimmed = tag.trim().to_lowercase();
                if trimmed == "u" || trimmed == "/u" || trimmed.starts_with("font ") || trimmed == "/font" {
                    out.push('<');
                    out.push_str(&tag);
                    out.push('>');
                } else {
                    out.push_str("&lt;");
                    out.push_str(&tag);
                    out.push_str("&gt;");
                }
            } else {
                out.push_str("&lt;");
                out.push_str(&tag);
            }
        } else {
            out.push(ch);
        }
    }
    out
}

pub fn safe_styled_text(text: &str) -> slint::StyledText {
    let prepared = prepare_markdown_for_slint(text);
    slint::StyledText::from_markdown(&prepared)
        .unwrap_or_else(|_| slint::StyledText::from_plain_text(text))
}

pub fn set_subtitle_window_text(sub: &SubtitleWindow, text: &str) {
    sub.set_subtitle_text(safe_styled_text(text));
    sub.set_subtitle_raw_text(text.into());
}

