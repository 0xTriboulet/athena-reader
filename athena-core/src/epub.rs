//! EPUB text extraction using the [`epub-stream`](https://crates.io/crates/epub-stream) crate.
//!
//! Provides a single convenience function [`extract_text_from_bytes`] that opens an
//! in-memory EPUB archive and concatenates the plain-text content of every spine
//! chapter into a single `String` suitable for playback in a reading session.

use std::io::Cursor;

use epub_stream::book::EpubBook;

/// Extracts plain text from an in-memory EPUB file.
///
/// The function opens the EPUB from `bytes` using [`EpubBook::from_reader`],
/// iterates over every chapter in spine order via [`EpubBook::chapter_text`],
/// and concatenates the results separated by newlines.
///
/// # Errors
///
/// Returns a human-readable error string if the EPUB cannot be parsed or if
/// all chapters are empty / unreadable.
pub fn extract_text_from_bytes(bytes: &[u8]) -> Result<String, String> {
    let cursor = Cursor::new(bytes);
    let mut book =
        EpubBook::from_reader(cursor).map_err(|e| format!("Failed to open EPUB: {e}"))?;

    let count = book.chapter_count();
    if count == 0 {
        return Err("EPUB contains no chapters.".to_string());
    }

    let mut full_text = String::new();
    for i in 0..count {
        match book.chapter_text(i) {
            Ok(text) => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    if !full_text.is_empty() {
                        full_text.push('\n');
                    }
                    full_text.push_str(trimmed);
                }
            }
            Err(_) => {
                // Skip unreadable chapters rather than failing entirely.
            }
        }
    }

    if full_text.is_empty() {
        return Err("No readable text found in EPUB.".to_string());
    }

    Ok(full_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Passing garbage bytes should return a descriptive error, not panic.
    #[test]
    fn invalid_bytes_returns_error() {
        let result = extract_text_from_bytes(b"this is not an epub");
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("Failed to open EPUB"),
            "unexpected error: {msg}"
        );
    }

    /// Empty input should fail gracefully.
    #[test]
    fn empty_bytes_returns_error() {
        let result = extract_text_from_bytes(b"");
        assert!(result.is_err());
    }
}
