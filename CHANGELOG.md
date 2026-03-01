# Changelog

All notable changes to this project will be documented in this file.

## [0.2.1] - 2026-03-01

### Fixed
- Suppress Windows console window in GUI binary by setting the `windows_subsystem = "windows"` attribute

## [0.2.0] - 2026-02-25

### Added
- EPUB import support via the Import flow
- Persisted paused-reading cache (text + position) restored on app restart
- Unified clipboard paste flow for text and images, with text preferred when both exist

### Changed
- Import now supports images, PDFs, `.epub`, and `.txt` files
- Preview double-click now pauses active playback before opening the editor
- Editor save now rebuilds/remaps the stream to the equivalent reading position after text edits
- Controller row spacing improved for clearer separation from the text section
- Documentation expanded for feature coverage and behavior details in `README.md` and `notes/specification.md`

## [0.1.0] - 2026-02-23

### Added
- Initial release of Athena Reader
- Native desktop speed-reading tool with OCR support
- Paste screenshot from clipboard and extract text via local OCR (offline after model cache)
- Stream extracted text at a configurable WPM using ORP-style centered word display
- Import images, PDFs, and `.txt` files
- Editable text before playback
- Playback controls: Play/Pause (Space), Step back/forward (Arrow keys), Restart (R)
- WPM, chunk size, and font size sliders
- Light, Dark, and High Contrast themes
- `athena-core`: OCR + text normalization/tokenization + reading session logic
- `athena-app`: egui/eframe GUI application
- Environment variable overrides for OCR model paths, download URLs, and cache directory
