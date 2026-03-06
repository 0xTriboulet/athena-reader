# Athena Reader Specification

## 1. Summary
Athena Reader is a native desktop speed-reading tool that lets a user paste or import content, extract text locally (OCR for images, embedded-text extraction for PDFs, chapter extraction for EPUBs, direct text-file import), edit extracted text, and present the text one word (or chunk) at a time at a configurable WPM. Paused reading position and text are persisted locally and restored on relaunch.

## Progress
_Historical implementation log; older entries may describe superseded intermediate decisions._

- 2026-03-06: Added Live View feature: a "+" button in the preview panel opens a separate OS window showing all tokens as flowing text with the current streaming word highlighted (theme-aware color + underline). The viewport smoothly auto-scrolls to keep the highlight in the top 1/3. Opening Live View pauses playback. TDD approach with 10 new unit tests. Tests run: `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`.
- 2026-02-25: Added unified clipboard paste support for text and images in the app control flow. The Paste control now prefers clipboard text when both text and image are present, and falls back to image OCR otherwise. Tests run: `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`.
- 2026-02-25: Improved editor-save position behavior: opening the editor from preview pauses playback, and Save remaps to the closest logical reading position in edited text (with fallback to index 0 when out of range). Tests run: `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`.
- 2026-02-25: Documentation completeness/readability pass for README and specification to align with current implementation details (import matrix, editor semantics, persistence semantics, constraints). Tests run: not required (docs-only updates).
- 2026-02-22: Plan approved; implementation not started yet. Decisions: Tauri desktop app, leptess OCR backend, English-only OCR for v1, preview does not need to be editable, and multi-word chunks up to 5 words will be supported. Next steps: research Tauri/leptess setup, scaffold project, and begin TDD on text/tokenization and reader timing. Tests run: none.
- 2026-02-22: Researched dependencies. Tauri recommends using `create-tauri-app` (sh <(curl https://create.tauri.app/sh)) and following its prerequisites guide; Tauri supports any frontend that compiles to HTML/JS/CSS. Leptess requires system packages: `libleptonica-dev`, `libtesseract-dev`, `clang`, and language data like `tesseract-ocr-eng`. Tests run: none.
- 2026-02-22: Scaffolded a minimal Tauri project (`src-tauri` with Cargo.toml, build.rs, main.rs, tauri.conf.json) and a placeholder `dist/index.html`. Ran `cargo test` in `src-tauri`; it failed because `glib-2.0` system library is missing. Attempted to install Tauri/leptess prerequisites via `sudo apt-get`, but sudo requires a password and installation could not proceed. Tests run: `cargo test` (failed: missing glib-2.0 pkg-config).
- 2026-02-22: Proceeding with a separate Rust library crate (`athena-core`) to implement text/tokenization and reader logic with TDD while the Tauri build is blocked by missing system dependencies. Tests run: none.
- 2026-02-22: Implemented `athena-core` text and reader modules with unit tests. Text module covers whitespace normalization, hyphenation removal, tokenization, and combined normalization/tokenization. Reader module includes interval calculation, navigation clamping, progress reporting, and playback state. Tests run: `cargo test` (athena-core) — 10 passed.
- 2026-02-22: Added `athena-core` OCR module with a stub engine plus an optional leptess-backed engine behind a `leptess` feature. The leptess implementation currently supports file-path input only and returns a clear error for byte input; OCR execution not validated because system Tesseract/Leptonica libraries are missing. Tests run: `cargo test` (athena-core) — 12 passed.
- 2026-02-22: Tauri build and UI wiring remain blocked pending installation of Linux system dependencies (e.g., glib/gtk/webkit). Next steps after deps are available: re-run `cargo test` in `src-tauri`, integrate `athena-core` into the Tauri backend, and wire the UI paste/OCR/playback flow. Tests run: none.
- 2026-02-22: Added multi-word chunk support (1-5 tokens) to the reader session via a configurable `chunk_size` and `current_chunk` helper. Tests run: `cargo test` (athena-core) — 15 passed.
- 2026-02-22: Added reader `set_wpm` helper and a minimal Tauri backend/HTML UI wiring. Tauri now exposes commands for OCR (path/bytes), session control (start/advance/rewind/restart), and playback settings (wpm/chunk size/is_playing). The UI includes paste-to-OCR handling, start/play controls, and progress display, but cannot be validated because Tauri system deps are still missing. Tests run: `cargo test` (athena-core) — 16 passed; `cargo test` (src-tauri) not run (missing glib/gtk/webkit deps).
- 2026-02-22: Attempted to install Tauri and leptess system dependencies without sudo (`apt-get update/install`), but lacked permissions to write APT lock/cache files. Tauri build remains blocked pending system deps. Tests run: none.
- 2026-02-22: Implemented OCR preprocessing in `athena-core` using the `image` crate (grayscale, contrast adjustment, thresholding) with unit tests, and added temp-file handling for leptess to support byte input once system deps are available. Tests run: `cargo test` (athena-core) — 18 passed.
- 2026-02-22: Added `athena-core` settings module (UserSettings + Theme enums) with JSON load/save helpers and unit tests. Defaults: 300 WPM, font size 32, dark theme. Tests run: `cargo test` (athena-core) — 21 passed.
- 2026-02-24: Added EPUB import support via the `epub-stream` crate. New `athena-core::epub` module extracts plain text from all spine chapters. Import file picker now accepts `.epub` files. Background extraction mirrors the PDF path. Tests run: `cargo clippy --workspace -- -D warnings`, `cargo test --workspace` — 24 passed.
- 2026-02-25: Added paused-session persistence. Whenever reading is paused, the app caches the current text and token index and restores that paused position on next launch. Tests run: `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`.
- 2026-02-22: Switching from Tauri/webview to a native Rust UI (egui/eframe) to meet the single static binary requirement. Tauri scaffolding is deprecated; next steps are to remove it, scaffold an eframe app, and wire athena-core directly into the UI. Tests run: none.
- 2026-02-22: Removed the Tauri scaffold and created a native `athena-app` (eframe) that uses `athena-core` directly. Implemented clipboard image paste (arboard), OCR invocation, reader controls, settings UI (WPM, chunk size, font size, theme), and playback timing. Tests run: `cargo test -p athena-core` (21 passed), `cargo test -p athena-app` (0 tests, build ok).
- 2026-02-22: OCR backend change requested: replace unmaintained `leptess` with an ONNX-based OCR engine (`ocrs` using RTen). Pending work: migrate athena-core OCR module, download/manage models, and update tests. Tests run: none.
- 2026-02-22: Migrated athena-core OCR to `ocrs` (RTen). Added model-path validation, OCR pipeline using `prepare_input`/`detect_words`/`find_text_lines`/`recognize_text`, and updated athena-app to load models from cache or env overrides. Tests run: `cargo test -p athena-core`, `cargo test -p athena-app`.
- 2026-02-22: Added OCR model auto-download/cache with env overrides, async OCR execution with explicit UI states, keyboard shortcuts, and status/UX polish. Switched default OCR models to MIT-licensed PaddleOCR ONNX artifacts from Hugging Face (`GetcharZp/go-ocr`) and execute them via `oar-ocr` (ONNX Runtime). Tests run: `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`.
- 2026-02-22: Implemented reading-mode UI upgrades: rectangular window, centered ORP-style word display (highlighted pivot character is stationary), bottom-left truncated OCR preview with double-click editor window, bottom-right status/progress/spinner overlay, and removed Start button (session is created automatically after OCR). Tests run: `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`.
- 2026-02-22: Added Import support for images and PDFs (PDFs use embedded text extraction via `pdf-extract`), made Import file picker non-blocking to avoid Linux “not responding” dialogs, reduced Preview truncation to 25 words, detached Edit Text into a separate OS window (egui viewport) with scrolling, and expanded UI ranges (WPM up to 900, font size up to 200). Tests run: `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`.

## 2. Goals
- Paste a screenshot directly into the app UI.
- Import an image, PDF, EPUB, or text file for local text extraction.
- Perform on-device OCR with a small model and no network calls.
- Allow editing extracted text before/while reading, without losing the reader's logical position.
- Present extracted text as a one-word stream at a target WPM (default 300).
- Provide responsive playback controls (play/pause, back/forward, WPM adjustment).

## 3. Non-Goals
- No cloud OCR or server-side processing.
- No user accounts, sync, or sharing.
- No document library (import is one-off).
- No OCR for scanned/image-only PDFs in v1 (PDF import extracts embedded text only).

## 4. Target Platform
- Native Rust UI application built with egui/eframe (no webview).
- Single binary distribution targeting Linux and Windows where possible.
- Offline-first; all processing occurs locally.

### 4.1 Build and Distribution
- Linux: `x86_64-unknown-linux-gnu` is the default development target; static linking is not guaranteed due to ONNX Runtime.
- Windows: `x86_64-pc-windows-msvc`.
- OCR models are downloaded/cached locally on demand (or can be provided via env vars); document any platform-specific steps.

## 5. Primary User Flow
1. User launches Athena Reader.
2. User imports an image/PDF/EPUB using Import, or uses Paste (Ctrl/Cmd+V) to paste clipboard text/image.
3. App runs OCR (for images), extracts embedded text (for PDFs), or extracts chapter text (for EPUBs) and shows a truncated preview of extracted text (first 25 words).
4. If desired, user double-clicks the preview to open an Edit Text window and edits the extracted text. If playback is active, the app pauses first.
5. App automatically prepares the reading session and shows the first word/chunk.
6. When edits are saved, the reading session is rebuilt from edited text and position is remapped to the closest logical point (fallback to start if no valid mapped position remains).
7. User presses Play (or Space) to begin word streaming.
8. While paused, the app persists current text and token index and restores that paused session on next launch.

## 6. Functional Requirements
### 6.1 Clipboard Input
- Accept plaintext from the system clipboard and start a reading session directly.
- Accept images from the system clipboard (PNG, JPEG, BMP) and run OCR.
- If both text and image exist in the clipboard, prefer text.
- If the clipboard does not contain usable text or an image, show a clear error state.

### 6.1.1 File Import
- Allow importing images (PNG/JPEG/etc), PDFs, EPUBs, and plain-text files via a file picker.
- Import must not freeze the UI event loop while the picker is open.
- For PDFs: extract embedded text; scanned/image-only PDFs may return empty text and should show an error.
- For EPUBs: extract plain text from all spine chapters using the `epub-stream` crate; concatenate in reading order.
- For `.txt`: decode from bytes with UTF-8 lossy conversion and trim surrounding whitespace.

### 6.2 OCR Pipeline
- Decode imported/clipboard image bytes into RGB pixels for OCR input.
- Use a local OCR backend with ONNX models (Latin alphabet).
- Produce plain text output and a confidence score (if provided by the backend).
- If OCR fails or returns no text, display an error with retry guidance.

### 6.3 Text Normalization and Tokenization
- Normalize whitespace and line breaks to single spaces.
- Preserve punctuation attached to its word (e.g., "word," shows as one token).
- Remove hyphenation across line breaks when possible.
- Generate an ordered token list for playback.

### 6.4 Reading Playback
- Display exactly one token at a time.
- Display is centered vertically/horizontally using ORP-style alignment, with a highlighted pivot character that stays stationary as words change.
- WPM interval: `interval_ms = 60_000 / WPM` (rounded to nearest ms).
- Controls:
  - Play/Pause (Space bar).
  - Step back/forward by 5 words.
  - Restart from beginning (and pause).
  - WPM slider (range 100-900; default 300).
  - Font size slider (range 18-200; default 32).
- Show progress (current index / total tokens).

### 6.5 UI States
- Idle (paste prompt).
- Processing (spinner + status) for OCR or PDF extraction.
- Result Preview (read-only, truncated preview text area).
- Reading Ready (first word/chunk visible; Play starts streaming).
- Reading Mode (word display, controls, progress).
- Error State (clipboard missing usable content, OCR failure, empty text).
- Edit Text Window (separate OS window; scrollable + editable full text; Save/Cancel).
- Live View Window (separate OS window; read-only flowing text with highlighted current word and smooth auto-scroll).

### 6.5.1 Edit Text behavior
- Opening Edit Text from preview captures reading context (current token + neighbors) for position remapping.
- If playback is running when editor is opened, playback pauses immediately.
- Saving editor text rebuilds the reading session in paused mode.
- Position remapping prefers the original logical position using token occurrence and surrounding context; if no valid position exists, reset to index 0.

### 6.5.2 Live View behavior
- A "+" button in the bottom-right of the preview header opens the Live View window.
- Opening the Live View pauses playback if currently playing.
- The Live View renders all tokens as flowing text with the current word (or chunk) highlighted using theme-aware color and underline.
- The viewport smoothly auto-scrolls to keep the highlighted word in the top one-third of the visible area.
- The Live View is read-only; edits must use the Edit Text window.
- Closing the Live View (via window close button) does not change playback state.

### 6.6 Settings and Persistence
- Persist WPM, chunk size, font size, and theme locally (app config file).
- Persist paused reading session state (text + current token index) locally and restore it on next app launch.
- Update paused-session cache whenever pause semantics occur (pause, restart, paused navigation, or save-edited paused session).
- Ignore/clear invalid cached sessions on startup; out-of-range token index restores to 0.

### 6.7 Accessibility
- Keyboard-only operation for core actions.
- Adjustable font size and high-contrast mode.
- Focus-visible controls and high-contrast-friendly visuals in native egui UI.

## 7. Data Model (In-Memory)
- `ReadingSession`:
  - `raw_text: String`
  - `tokens: Vec<String>`
  - `current_index: usize`
  - `wpm: u32`
  - `chunk_size: usize`
  - `is_playing: bool`
- `UserSettings`:
  - `wpm: u32`
  - `chunk_size: usize`
  - `font_size: u32`
  - `theme: "light" | "dark" | "high-contrast"`
- `ReadingCache` (persisted JSON snapshot):
  - `text: String`
  - `current_index: usize`

## 8. Architecture
- **Native UI (egui/eframe)**: windowing, layout, keyboard shortcuts, and paste handling.
- **Rust Core**:
  - `ocr` module: preprocess + OCR backend.
  - `text` module: normalization + tokenization.
  - `reader` module: playback state and timing.
  - `settings` module: persisted user preferences and paused-session cache.
- **App State**: single-process state management; no IPC required.

### OCR Backend
- `oar-ocr` (ONNX-based, executed via ONNX Runtime) as the primary backend.
- Default models sourced from Hugging Face `GetcharZp/go-ocr` (MIT) and cached locally.

### OCR Backend Migration (Leptess -> ONNX PaddleOCR)
- Remove `leptess` dependencies and feature flags from `athena-core` and `athena-app`.
- Add `oar-ocr` dependency and load detection/recognition ONNX models plus a character dictionary.
- Download or bundle models from Hugging Face; cache in the app config/cache directory.
- Models auto-download on demand into the cache directory unless `ATHENA_OCR_DISABLE_DOWNLOAD` is set; URLs and SHA-256 hashes can be overridden via env vars.
- Default lookup filenames in the cache directory: `det.onnx`, `rec.onnx`, `dict.txt`.
- Override paths with:
  - `ATHENA_OCR_DETECTION_MODEL`
  - `ATHENA_OCR_RECOGNITION_MODEL`
  - `ATHENA_OCR_DICT_PATH`
- Override download sources and integrity pins with:
  - `ATHENA_OCR_DETECTION_URL` / `ATHENA_OCR_DETECTION_SHA256`
  - `ATHENA_OCR_RECOGNITION_URL` / `ATHENA_OCR_RECOGNITION_SHA256`
  - `ATHENA_OCR_DICT_URL` / `ATHENA_OCR_DICT_SHA256`
- Update tests to cover model download/path errors and basic preprocessing.

## 9. Performance Requirements
- OCR generally completes within a few seconds for typical screenshots (depends on image size/quality and hardware).
- Word playback timing accuracy within ±50 ms.
- UI remains responsive during OCR/import via background processing.

## 10. Privacy and Security
- No network traffic during OCR execution; OCR runs locally.
- On first use (or when models are missing), the app may download OCR models from Hugging Face unless downloads are disabled or models are provided locally.
- Settings and paused reading state are persisted locally; OCR/image processing stays local.
- Optional diagnostics stored locally with user opt-in (future).

## 11. Validation and Testing
- Current automated coverage includes unit tests for:
  - Text normalization/tokenization
  - Reader timing/navigation/chunking
  - Settings and paused-reading cache persistence
  - EPUB extraction error handling
  - Editor remap and restore behavior in app-layer tests
- Future improvements:
  - Integration tests for OCR pipeline on fixture images
  - Extended manual UX testing for keyboard workflows and failure states

## 12. Resolved Product Decisions
- Should multi-language OCR be supported in v1?
  USER: English only for version 1 is acceptable.
- Is a non-editable preview acceptable, or is inline editing required?
  USER: The main preview stays read-only, but double-click opens an editable text window and saved edits are used for playback.
- Should the app support multi-word chunks (2-3 words) as an optional mode?
  USER: Yes, there should be a setting for multi-word chunks. Upto 5.
