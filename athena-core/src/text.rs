//! Text normalization and tokenization utilities.
//!
//! OCR output (and PDF extracted text) tends to contain odd whitespace and line breaks.
//! This module provides simple normalization and splitting into tokens used by
//! [`crate::reader::ReadingSession`].

/// Normalizes raw extracted text for playback.
///
/// - Collapses all whitespace into single spaces
/// - Removes soft hyphenation across line breaks for alphabetic words (e.g. `"hy-\nphen"` → `"hyphen"`)
pub fn normalize_text(input: &str) -> String {
    let mut output = String::new();
    let mut chars = input.chars().peekable();
    let mut last_was_space = false;
    let mut prev_is_alpha = false;

    while let Some(ch) = chars.next() {
        if ch == '-' && prev_is_alpha {
            let mut lookahead = chars.clone();
            let mut saw_newline = false;
            match lookahead.peek() {
                Some('\r') => {
                    lookahead.next();
                    if lookahead.peek() == Some(&'\n') {
                        lookahead.next();
                    }
                    saw_newline = true;
                }
                Some('\n') => {
                    lookahead.next();
                    saw_newline = true;
                }
                _ => {}
            }

            if saw_newline
                && let Some(next_char) = lookahead.peek()
                && next_char.is_alphabetic()
            {
                if chars.peek() == Some(&'\r') {
                    chars.next();
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                } else if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                continue;
            }
        }

        if ch.is_whitespace() {
            if !last_was_space && !output.is_empty() {
                output.push(' ');
                last_was_space = true;
            }
            prev_is_alpha = false;
            continue;
        }

        output.push(ch);
        last_was_space = false;
        prev_is_alpha = ch.is_alphabetic();
    }

    output
}

/// Splits normalized text into tokens for streaming.
///
/// This uses whitespace splitting and preserves punctuation inside tokens.
pub fn tokenize(input: &str) -> Vec<String> {
    input
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
}

/// Convenience wrapper: [`normalize_text`] followed by [`tokenize`].
pub fn normalize_and_tokenize(input: &str) -> Vec<String> {
    tokenize(&normalize_text(input))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_collapses_whitespace() {
        let input = "Hello\n\n   world\tfrom  Athena";
        let normalized = normalize_text(input);
        assert_eq!(normalized, "Hello world from Athena");
    }

    #[test]
    fn normalize_removes_hyphenation_across_lines() {
        let input = "hy-\nphen and co-\noperate";
        let normalized = normalize_text(input);
        assert_eq!(normalized, "hyphen and cooperate");
    }

    #[test]
    fn tokenize_preserves_punctuation() {
        let tokens = tokenize("Hello, world! This is Athena.");
        assert_eq!(tokens, vec!["Hello,", "world!", "This", "is", "Athena."]);
    }

    #[test]
    fn normalize_and_tokenize_combines_steps() {
        let input = "Speed-\nreading\nworks.";
        let tokens = normalize_and_tokenize(input);
        assert_eq!(tokens, vec!["Speedreading", "works."]);
    }
}
