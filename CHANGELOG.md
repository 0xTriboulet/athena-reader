# Changelog

All notable changes to this project will be documented in this file.

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
