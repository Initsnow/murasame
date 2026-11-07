pub mod callbacks;
pub mod format;
pub mod interaction;
pub mod stream;

pub use callbacks::*;
pub use format::*;
pub use interaction::*;
pub use stream::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::SubtitleWindow;

    #[test]
    fn test_set_subtitle_window_text_markdown_sync() {
        let _ = slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions {
                mock_time: false,
                threading: true,
                ..Default::default()
            },
        )));

        let sub = match SubtitleWindow::new() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Skipping SubtitleWindow UI assertions (backend unavailable): {e}");
                return;
            }
        };
        let sample_md = "**丛雨**: *你好*，[Slint](https://slint.dev)！";
        set_subtitle_window_text(&sub, sample_md);

        assert_eq!(sub.get_subtitle_raw_text(), sample_md);
        assert_eq!(sub.get_subtitle_text(), safe_styled_text(sample_md));

        // Test clearing subtitle
        set_subtitle_window_text(&sub, "");
        assert_eq!(sub.get_subtitle_raw_text(), "");
        assert_eq!(sub.get_subtitle_text(), safe_styled_text(""));
    }

    #[test]
    fn test_safe_styled_text_markdown_cases() {
        let test_cases = vec![
            ("Headings", "# Heading 1\n## Heading 2"),
            ("Blockquote", "> This is a quote"),
            ("Code block", "```markdown\n**粗体**\n```"),
            ("HTML tags", "<think>reasoning</think>Hello <u>world</u>!"),
            ("Inline code", "`code`"),
            (
                "User exact case",
                "用两侧双星号包住文字：\n\n```markdown\n**粗体**\n```\n\n或两侧双下划线：\n\n```markdown\n__粗体__\n```\n\n渲染后是：**粗体**",
            ),
        ];
        for (name, tc) in test_cases {
            let styled = safe_styled_text(tc);
            assert_ne!(styled, slint::StyledText::default(), "Failed for case: {name}");
        }
    }
}
