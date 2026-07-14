//! Unicode-safe text helpers for terminal previews and bounded output.

/// Keep at most `max_chars` Unicode scalar values and append `suffix` only
/// when the input was actually shortened.
pub fn truncate(text: &str, max_chars: usize, suffix: &str) -> String {
    match text.char_indices().nth(max_chars) {
        Some((byte_index, _)) => format!("{}{}", &text[..byte_index], suffix),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn truncates_multibyte_text_without_splitting_a_character() {
        assert_eq!(truncate("Habari 👋🏾 dunia", 8, "…"), "Habari 👋…");
    }

    #[test]
    fn leaves_short_text_unchanged() {
        assert_eq!(truncate("こんにちは", 5, "…"), "こんにちは");
    }
}
