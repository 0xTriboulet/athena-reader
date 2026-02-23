//! Reading session state and playback timing.
//!
//! The GUI drives a [`ReadingSession`] forward based on a WPM-derived tick interval.

/// Holds the token stream and playback state for the speed-reading UI.
#[derive(Debug, Clone)]
pub struct ReadingSession {
    /// Original extracted text (OCR/PDF/editor) for preview/editing.
    pub raw_text: String,
    /// Tokenized representation used for streaming.
    pub tokens: Vec<String>,
    /// Current token index within `tokens`.
    pub current_index: usize,
    /// Words-per-minute for playback.
    pub wpm: u32,
    /// Number of tokens to show at once (chunking).
    pub chunk_size: usize,
    /// Whether playback is currently advancing on ticks.
    pub is_playing: bool,
}

impl ReadingSession {
    /// Creates a new session starting at the first token.
    pub fn new(raw_text: String, tokens: Vec<String>, wpm: u32) -> Self {
        Self {
            raw_text,
            tokens,
            current_index: 0,
            wpm,
            chunk_size: 1,
            is_playing: false,
        }
    }

    /// Returns the current token (single-word view), if any.
    pub fn current_token(&self) -> Option<&str> {
        self.tokens.get(self.current_index).map(String::as_str)
    }

    /// Returns the currently displayed chunk of tokens.
    ///
    /// Chunking is clamped by the remaining number of tokens.
    pub fn current_chunk(&self) -> Vec<&str> {
        if self.tokens.is_empty() {
            return Vec::new();
        }
        let end_index = (self.current_index + self.chunk_size).min(self.tokens.len());
        self.tokens[self.current_index..end_index]
            .iter()
            .map(String::as_str)
            .collect()
    }

    /// Advances the session by `count` tokens, clamping at the end, and returns the new current token.
    pub fn advance(&mut self, count: usize) -> Option<&str> {
        if self.tokens.is_empty() {
            return None;
        }
        let last_index = self.tokens.len() - 1;
        self.current_index = (self.current_index + count).min(last_index);
        self.current_token()
    }

    /// Rewinds the session by `count` tokens, clamping at the start, and returns the new current token.
    pub fn rewind(&mut self, count: usize) -> Option<&str> {
        if self.tokens.is_empty() {
            return None;
        }
        self.current_index = self.current_index.saturating_sub(count);
        self.current_token()
    }

    /// Resets playback position to the first token.
    pub fn restart(&mut self) {
        self.current_index = 0;
    }

    /// Sets whether the session should advance on ticks.
    pub fn set_playing(&mut self, is_playing: bool) {
        self.is_playing = is_playing;
    }

    /// Updates the session WPM (used to compute tick interval).
    pub fn set_wpm(&mut self, wpm: u32) {
        self.wpm = wpm;
    }

    /// Sets the chunk size, clamped to the UI-supported range.
    pub fn set_chunk_size(&mut self, chunk_size: usize) {
        self.chunk_size = chunk_size.clamp(1, 5);
    }

    /// Returns `(current_index, total_tokens)` as a 1-based index for display.
    pub fn progress(&self) -> (usize, usize) {
        if self.tokens.is_empty() {
            return (0, 0);
        }
        (self.current_index + 1, self.tokens.len())
    }
}

/// Computes the playback tick interval (milliseconds) for a given WPM.
///
/// Returns `None` when `wpm == 0`.
pub fn interval_ms(wpm: u32) -> Option<u64> {
    if wpm == 0 {
        return None;
    }
    let wpm = wpm as u64;
    Some((60_000 + wpm / 2) / wpm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_ms_returns_none_for_zero() {
        assert_eq!(interval_ms(0), None);
    }

    #[test]
    fn interval_ms_rounds_to_nearest() {
        assert_eq!(interval_ms(300), Some(200));
        assert_eq!(interval_ms(333), Some(180));
    }

    #[test]
    fn session_navigation_clamps_bounds() {
        let mut session = ReadingSession::new(
            "one two three".to_string(),
            vec!["one".into(), "two".into(), "three".into()],
            300,
        );

        assert_eq!(session.current_token(), Some("one"));
        session.advance(1);
        assert_eq!(session.current_token(), Some("two"));
        session.advance(10);
        assert_eq!(session.current_token(), Some("three"));
        session.rewind(10);
        assert_eq!(session.current_token(), Some("one"));
        session.restart();
        assert_eq!(session.current_token(), Some("one"));
    }

    #[test]
    fn progress_reports_zero_for_empty() {
        let session = ReadingSession::new(String::new(), Vec::new(), 250);
        assert_eq!(session.progress(), (0, 0));
    }

    #[test]
    fn progress_reports_one_based_index() {
        let mut session = ReadingSession::new(
            "alpha beta".to_string(),
            vec!["alpha".into(), "beta".into()],
            250,
        );
        assert_eq!(session.progress(), (1, 2));
        session.advance(1);
        assert_eq!(session.progress(), (2, 2));
    }

    #[test]
    fn set_playing_updates_state() {
        let mut session = ReadingSession::new(String::new(), Vec::new(), 250);
        assert!(!session.is_playing);
        session.set_playing(true);
        assert!(session.is_playing);
    }

    #[test]
    fn set_wpm_updates_value() {
        let mut session = ReadingSession::new(String::new(), Vec::new(), 250);
        session.set_wpm(400);
        assert_eq!(session.wpm, 400);
    }

    #[test]
    fn chunk_size_defaults_to_one() {
        let session = ReadingSession::new(String::new(), Vec::new(), 300);
        assert_eq!(session.chunk_size, 1);
    }

    #[test]
    fn chunk_size_clamps_to_bounds() {
        let mut session = ReadingSession::new(String::new(), Vec::new(), 300);
        session.set_chunk_size(0);
        assert_eq!(session.chunk_size, 1);
        session.set_chunk_size(7);
        assert_eq!(session.chunk_size, 5);
    }

    #[test]
    fn current_chunk_returns_sized_slice() {
        let mut session = ReadingSession::new(
            "alpha beta gamma delta".to_string(),
            vec![
                "alpha".into(),
                "beta".into(),
                "gamma".into(),
                "delta".into(),
            ],
            300,
        );
        session.set_chunk_size(3);
        assert_eq!(session.current_chunk(), vec!["alpha", "beta", "gamma"]);
        session.advance(2);
        assert_eq!(session.current_chunk(), vec!["gamma", "delta"]);
    }
}
