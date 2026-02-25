//! Settings model and persistence.
//!
//! The GUI stores user-adjustable settings (WPM, font size, theme, etc) in a JSON file.

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Visual theme options for the GUI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Theme {
    /// Light theme (dark text).
    Light,
    /// Dark theme (light text).
    Dark,
    /// High-contrast theme intended for accessibility.
    HighContrast,
}

/// Persisted user preferences.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserSettings {
    /// Words per minute for playback.
    pub wpm: u32,
    /// Font size used in the central ORP display.
    pub font_size: u32,
    /// Number of words displayed per tick.
    pub chunk_size: usize,
    /// Theme selection.
    pub theme: Theme,
}

/// Persisted paused reading session snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadingCache {
    /// Full text currently loaded in the reader.
    pub text: String,
    /// Zero-based token index currently displayed.
    pub current_index: usize,
}

impl Default for UserSettings {
    /// Returns default settings used on first run.
    fn default() -> Self {
        Self {
            wpm: 300,
            font_size: 32,
            chunk_size: 1,
            theme: Theme::Dark,
        }
    }
}

/// Errors returned when loading or saving settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsError {
    /// Underlying filesystem error.
    Io(String),
    /// JSON parse/serialization error.
    Parse(String),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingsError::Io(message) => write!(f, "Settings IO error: {message}"),
            SettingsError::Parse(message) => write!(f, "Settings parse error: {message}"),
        }
    }
}

impl std::error::Error for SettingsError {}

/// Loads settings from the given path.
///
/// If the file does not exist, defaults are returned.
pub fn load_settings(path: &Path) -> Result<UserSettings, SettingsError> {
    if !path.exists() {
        return Ok(UserSettings::default());
    }
    let contents =
        std::fs::read_to_string(path).map_err(|error| SettingsError::Io(error.to_string()))?;
    serde_json::from_str(&contents).map_err(|error| SettingsError::Parse(error.to_string()))
}

/// Saves settings to the given path, creating parent directories as needed.
pub fn save_settings(path: &Path, settings: &UserSettings) -> Result<(), SettingsError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| SettingsError::Io(error.to_string()))?;
    }
    let contents = serde_json::to_string_pretty(settings)
        .map_err(|error| SettingsError::Parse(error.to_string()))?;
    std::fs::write(path, contents).map_err(|error| SettingsError::Io(error.to_string()))
}

/// Loads the paused reading cache from disk.
///
/// Returns `Ok(None)` when no cache file exists.
pub fn load_reading_cache(path: &Path) -> Result<Option<ReadingCache>, SettingsError> {
    if !path.exists() {
        return Ok(None);
    }
    let contents =
        std::fs::read_to_string(path).map_err(|error| SettingsError::Io(error.to_string()))?;
    let cache =
        serde_json::from_str(&contents).map_err(|error| SettingsError::Parse(error.to_string()))?;
    Ok(Some(cache))
}

/// Saves the paused reading cache to disk, creating parent directories as needed.
pub fn save_reading_cache(path: &Path, cache: &ReadingCache) -> Result<(), SettingsError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| SettingsError::Io(error.to_string()))?;
    }
    let contents = serde_json::to_string_pretty(cache)
        .map_err(|error| SettingsError::Parse(error.to_string()))?;
    std::fs::write(path, contents).map_err(|error| SettingsError::Io(error.to_string()))
}

/// Removes the paused reading cache file when it exists.
pub fn clear_reading_cache(path: &Path) -> Result<(), SettingsError> {
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(path).map_err(|error| SettingsError::Io(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_settings_returns_default_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = load_settings(&path).unwrap();
        assert_eq!(settings, UserSettings::default());
    }

    #[test]
    fn save_and_load_settings_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = UserSettings {
            wpm: 420,
            font_size: 28,
            chunk_size: 3,
            theme: Theme::HighContrast,
        };
        save_settings(&path, &settings).unwrap();
        let loaded = load_settings(&path).unwrap();
        assert_eq!(loaded, settings);
    }

    #[test]
    fn load_settings_rejects_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "not-json").unwrap();
        let result = load_settings(&path);
        assert!(matches!(result, Err(SettingsError::Parse(_))));
    }

    #[test]
    fn load_reading_cache_returns_none_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reading_cache.json");
        let cache = load_reading_cache(&path).unwrap();
        assert_eq!(cache, None);
    }

    #[test]
    fn save_and_load_reading_cache_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reading_cache.json");
        let cache = ReadingCache {
            text: "hello world".to_string(),
            current_index: 1,
        };
        save_reading_cache(&path, &cache).unwrap();
        let loaded = load_reading_cache(&path).unwrap();
        assert_eq!(loaded, Some(cache));
    }

    #[test]
    fn load_reading_cache_rejects_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reading_cache.json");
        std::fs::write(&path, "not-json").unwrap();
        let result = load_reading_cache(&path);
        assert!(matches!(result, Err(SettingsError::Parse(_))));
    }

    #[test]
    fn clear_reading_cache_removes_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reading_cache.json");
        std::fs::write(&path, "{}").unwrap();
        clear_reading_cache(&path).unwrap();
        assert!(!path.exists());
    }
}
