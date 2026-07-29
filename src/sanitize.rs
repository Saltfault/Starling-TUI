use anyhow::Context;

pub const MAX_NAME_BYTES: usize = 64;
pub const MAX_CODE_BYTES: usize = 512;

/// Normalizes line endings and removes terminal/control characters while preserving tabs/newlines.
pub fn text(input: &str, max_bytes: usize) -> String {
    let normalized = input.replace("\r\n", "\n").replace('\r', "\n");
    let clean: String = normalized
        .chars()
        .filter(|&ch| ch == '\n' || ch == '\t' || !ch.is_control())
        .collect();
    truncate_utf8(&clean, max_bytes).to_owned()
}

/// Sanitizes single-line user input, replacing line endings with spaces.
pub fn single_line(input: &str, max_bytes: usize) -> String {
    text(input, max_bytes)
        .replace(['\n', '\t'], " ")
        .trim()
        .to_owned()
}

/// Returns the longest prefix no larger than `max_bytes` ending on a UTF-8 boundary.
pub fn truncate_utf8(input: &str, max_bytes: usize) -> &str {
    if input.len() <= max_bytes {
        return input;
    }
    let mut end = max_bytes;
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    &input[..end]
}

pub fn sanitize_message(raw: &str) -> String {
    text(raw, starling::protocol::MAX_BODY_BYTES)
}

pub fn sanitize_name(raw: &str) -> String {
    single_line(raw, MAX_NAME_BYTES)
}

pub fn sanitize_code(raw: &str) -> Option<String> {
    let value: String = raw
        .trim()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
        .map(|character| character.to_ascii_uppercase())
        .collect();
    (!value.is_empty() && value.len() <= MAX_CODE_BYTES).then_some(value)
}

pub fn invite(input: &str) -> anyhow::Result<String> {
    sanitize_code(input).context("invite is empty, too large, or has no valid characters")
}

#[cfg(test)]
mod tests {
    use super::{MAX_CODE_BYTES, invite, single_line, text, truncate_utf8};

    #[test]
    fn text_normalizes_crlf_and_strips_controls() {
        assert_eq!(text("a\r\nb\rc\0\x1b[31m\t!", 100), "a\nb\nc[31m\t!");
    }

    #[test]
    fn truncation_obeys_bytes_and_utf8_boundaries() {
        assert_eq!(truncate_utf8("a🪶b", 4), "a");
        assert_eq!(truncate_utf8("a🪶b", 5), "a🪶");
        assert_eq!(text("🪶🪶", 7), "🪶");
    }

    #[test]
    fn single_line_removes_line_breaks_and_controls() {
        assert_eq!(single_line(" hello\r\nworld\0 ", 100), "hello world");
    }

    #[test]
    fn invite_normalizes_valid_alphabet() {
        assert_eq!(invite(" 01abEF ").unwrap(), "01ABEF");
    }

    #[test]
    fn invite_rejects_empty_oversize_and_invalid_alphabet() {
        assert!(invite("  ").unwrap_err().to_string().contains("empty"));
        assert!(
            invite(&"A".repeat(MAX_CODE_BYTES + 1))
                .unwrap_err()
                .to_string()
                .contains("large")
        );
        assert_eq!(invite("BIRD-123").unwrap(), "BIRD-123");
        assert!(invite("🪶").is_err());
    }
}
