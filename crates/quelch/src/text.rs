/// Truncate text to a maximum number of Unicode scalar values for display.
///
/// This avoids panics from slicing UTF-8 strings on non-character boundaries.
pub fn truncate_for_display(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();

    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_for_display;

    #[test]
    fn leaves_short_text_unchanged() {
        assert_eq!(truncate_for_display("hello", 10), "hello");
    }

    #[test]
    fn truncates_ascii_text() {
        assert_eq!(truncate_for_display("hello world", 5), "hello...");
    }

    #[test]
    fn truncates_utf8_without_panicking() {
        assert_eq!(truncate_for_display("abc\u{00a0}def", 4), "abc\u{00a0}...");
    }
}
