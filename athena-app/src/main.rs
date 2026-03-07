//! Athena Reader GUI application.
//!
//! This binary provides an egui/eframe UI that can:
//! - Paste text or an image from the clipboard
//! - Import images, PDFs, EPUBs, or text files
//! - Preview and edit extracted text
//! - Stream words/chunks at a target WPM using an ORP-style centered display

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::env;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::mpsc::TryRecvError;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use arboard::Clipboard;
use athena_core::ocr::{
    DEFAULT_DETECTION_FILENAME, DEFAULT_DETECTION_URL, DEFAULT_DICT_FILENAME, DEFAULT_DICT_URL,
    DEFAULT_RECOGNITION_FILENAME, DEFAULT_RECOGNITION_URL, OarOcrEngine, OcrEngine, OcrInput,
    OcrModelDownloadConfig, OcrModelDownloadInfo, OcrModelPaths, OcrResult, ensure_models,
};
use athena_core::reader::{ReadingSession, interval_ms};
use athena_core::settings::{
    ReadingCache, Theme, UserSettings, clear_reading_cache, load_reading_cache, load_settings,
    save_reading_cache, save_settings,
};
use athena_core::text;
use directories::ProjectDirs;
use eframe::egui;
use image::{DynamicImage, ImageBuffer, Rgba};

const ENV_OCR_DETECTION_MODEL: &str = "ATHENA_OCR_DETECTION_MODEL";
const ENV_OCR_RECOGNITION_MODEL: &str = "ATHENA_OCR_RECOGNITION_MODEL";
const ENV_OCR_DICT_PATH: &str = "ATHENA_OCR_DICT_PATH";
const ENV_OCR_DETECTION_URL: &str = "ATHENA_OCR_DETECTION_URL";
const ENV_OCR_RECOGNITION_URL: &str = "ATHENA_OCR_RECOGNITION_URL";
const ENV_OCR_DICT_URL: &str = "ATHENA_OCR_DICT_URL";
const ENV_OCR_DETECTION_SHA256: &str = "ATHENA_OCR_DETECTION_SHA256";
const ENV_OCR_RECOGNITION_SHA256: &str = "ATHENA_OCR_RECOGNITION_SHA256";
const ENV_OCR_DICT_SHA256: &str = "ATHENA_OCR_DICT_SHA256";
const ENV_OCR_CACHE_DIR: &str = "ATHENA_OCR_MODEL_CACHE_DIR";
const ENV_OCR_DISABLE_DOWNLOAD: &str = "ATHENA_OCR_DISABLE_DOWNLOAD";

/// Maximum number of words shown in the Preview panel.
const OCR_PREVIEW_WORD_LIMIT: usize = 25;

/// High-level UI state used to enable/disable controls and show status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UiState {
    /// Waiting for user input (no active session).
    Idle,
    /// Background processing (OCR or PDF text extraction) is running.
    Processing,
    /// Extracted text is available for preview/editing.
    Preview,
    /// Reading session exists (playing or paused).
    Reading,
    /// An unrecoverable error occurred (until next successful import/paste).
    Error,
}

impl UiState {}

/// Background result from OCR or PDF text extraction.
enum OcrResponse {
    /// Text successfully extracted.
    Success(String),
    /// A user-visible error message.
    Error(String),
}

/// Background result from the Import file picker.
enum ImportResponse {
    /// User canceled the dialog.
    Canceled,
    /// File selected; `bytes` contains file contents and flags control processing path.
    Selected {
        bytes: Vec<u8>,
        is_pdf: bool,
        is_txt: bool,
        is_epub: bool,
    },
}

/// Captures user intent from the editor UI.
#[derive(Debug, Default, Clone, Copy)]
struct EditorAction {
    /// Save the draft back to the main app state.
    save: bool,
    /// Close without saving.
    cancel: bool,
}

/// Captures the reading position context at the moment the editor is opened.
#[derive(Debug, Clone)]
struct EditAnchor {
    token: String,
    token_occurrence: usize,
    old_index: usize,
    prev_token: Option<String>,
    next_token: Option<String>,
}

/// Pre-computed text and token-offset tables for the Live View.
///
/// Rebuilt only when the session tokens change (new import, editor save, etc.),
/// NOT every frame. Stored behind an `Arc` so the deferred viewport callback
/// can reference it without cloning the text.
struct LiveViewTextData {
    /// Full text with space-separated tokens.
    full_text: String,
    /// Byte offset of each token's start in `full_text`.
    token_byte_starts: Vec<usize>,
    /// Byte offset just past each token's end in `full_text`.
    token_byte_ends: Vec<usize>,
    /// Character offset of each token's start in `full_text`.
    token_char_starts: Vec<usize>,
}

/// Shared state between the parent frame and the deferred Live View viewport.
///
/// The parent writes the latest session/settings snapshot each frame; the
/// deferred viewport callback reads it to render.
struct LiveViewShared {
    /// Pre-computed text data (rebuilt when tokens change).
    text_data: Option<Arc<LiveViewTextData>>,
    /// Current highlight start token index.
    current_index: usize,
    /// Number of tokens to highlight.
    chunk_size: usize,
    /// UI theme for highlight color selection.
    theme: Theme,
    /// Font size for text rendering (controlled by the Live View slider).
    font_size: f32,
    /// Set by the deferred callback when the user closes the Live View window.
    close_requested: bool,
    /// Stable leading edge of the sliding token window (persists across frames).
    /// Only reset when the highlight moves far outside the current window.
    window_start: usize,
}

impl Default for LiveViewShared {
    fn default() -> Self {
        Self {
            text_data: None,
            current_index: 0,
            chunk_size: 1,
            theme: Theme::Dark,
            font_size: 32.0,
            close_requested: false,
            window_start: 0,
        }
    }
}

/// Top-level egui application state for Athena Reader.
struct AthenaApp {
    ui_state: UiState,
    status: String,
    ocr_text: String,
    ocr_editor_open: bool,
    ocr_editor_draft: String,
    live_view_open: bool,
    /// Shared state for the deferred Live View viewport.
    live_view_shared: Arc<Mutex<LiveViewShared>>,
    /// Raw text from which the current `LiveViewTextData` was built (change detection).
    live_view_text_source: String,
    session: Option<ReadingSession>,
    model_paths: Option<OcrModelPaths>,
    model_download: OcrModelDownloadConfig,
    settings: UserSettings,
    settings_path: Option<PathBuf>,
    reading_cache_path: Option<PathBuf>,
    next_tick: Option<Instant>,
    ocr_rx: Option<mpsc::Receiver<OcrResponse>>,
    ocr_started_at: Option<Instant>,
    import_rx: Option<mpsc::Receiver<ImportResponse>>,
    edit_anchor: Option<EditAnchor>,
}

impl AthenaApp {
    /// Creates the application state, loads persisted settings, and configures egui visuals.
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Ensure child viewports open as separate OS windows when supported by the backend.
        cc.egui_ctx.set_embed_viewports(false);

        let settings_path = settings_path();
        let reading_cache_path = reading_cache_path();
        let settings = settings_path
            .as_ref()
            .and_then(|path| load_settings(path).ok())
            .unwrap_or_default();
        apply_theme(&cc.egui_ctx, &settings.theme);

        let mut ui_state = UiState::Idle;
        let mut status = "Paste text or an image to get started.".to_string();
        let mut ocr_text = String::new();
        let mut session = None;

        if let Some(path) = reading_cache_path.as_ref() {
            match load_reading_cache(path) {
                Ok(Some(cache)) => {
                    if let Some((restored_text, restored_session)) =
                        build_restored_session(&cache.text, &settings, cache.current_index)
                    {
                        ocr_text = restored_text;
                        session = Some(restored_session);
                        ui_state = UiState::Reading;
                        status = "Restored paused session.".to_string();
                    } else {
                        let _ = clear_reading_cache(path);
                    }
                }
                Ok(None) => {}
                Err(_) => {}
            }
        }

        Self {
            ui_state,
            status,
            ocr_text,
            ocr_editor_open: false,
            ocr_editor_draft: String::new(),
            live_view_open: false,
            live_view_shared: Arc::new(Mutex::new(LiveViewShared::default())),
            live_view_text_source: String::new(),
            session,
            model_paths: default_model_paths(),
            model_download: model_download_config(),
            settings,
            settings_path,
            reading_cache_path,
            next_tick: None,
            ocr_rx: None,
            ocr_started_at: None,
            import_rx: None,
            edit_anchor: None,
        }
    }

    fn persist_settings(&mut self) {
        if let Some(path) = self.settings_path.as_ref()
            && let Err(error) = save_settings(path, &self.settings)
        {
            self.set_status(format!("Failed to save settings: {error}"));
        }
    }

    fn persist_paused_reading_cache(&mut self) {
        let Some(path) = self.reading_cache_path.as_ref() else {
            return;
        };

        let Some(session) = self.session.as_ref() else {
            return;
        };

        if session.tokens.is_empty() || self.ocr_text.trim().is_empty() {
            if let Err(error) = clear_reading_cache(path) {
                self.set_status(format!("Failed to clear reading cache: {error}"));
            }
            return;
        }

        let cache = ReadingCache {
            text: self.ocr_text.clone(),
            current_index: session
                .current_index
                .min(session.tokens.len().saturating_sub(1)),
        };
        if let Err(error) = save_reading_cache(path, &cache) {
            self.set_status(format!("Failed to save reading cache: {error}"));
        }
    }

    fn clear_paused_reading_cache(&mut self) {
        let Some(path) = self.reading_cache_path.as_ref() else {
            return;
        };
        if let Err(error) = clear_reading_cache(path) {
            self.set_status(format!("Failed to clear reading cache: {error}"));
        }
    }

    /// Updates the status string shown in the bottom-right overlay.
    fn set_status(&mut self, message: impl Into<String>) {
        self.status = message.into();
    }

    /// Updates both the UI state and the status string.
    fn set_state(&mut self, state: UiState, message: impl Into<String>) {
        self.ui_state = state;
        self.status = message.into();
    }

    /// Transitions to the error state and updates the status string.
    fn set_error(&mut self, message: impl Into<String>) {
        self.ui_state = UiState::Error;
        self.status = message.into();
    }

    /// Spawns an OCR worker thread to keep the UI responsive.
    fn spawn_ocr(&mut self, bytes: Vec<u8>) {
        if self.ui_state == UiState::Processing {
            self.set_status("Processing already running.");
            return;
        }

        let Some(paths) = self.model_paths.as_ref() else {
            self.set_error(
        "OCR model paths not configured. Set ATHENA_OCR_DETECTION_MODEL, ATHENA_OCR_RECOGNITION_MODEL, ATHENA_OCR_DICT_PATH, or ATHENA_OCR_MODEL_CACHE_DIR.",
      );
            return;
        };

        let (tx, rx) = mpsc::channel();
        let paths = OcrModelPaths::new(
            paths.detection.clone(),
            paths.recognition.clone(),
            paths.dict.clone(),
        );
        let download_config = self.model_download.clone();
        let missing_models =
            !paths.detection.exists() || !paths.recognition.exists() || !paths.dict.exists();
        if missing_models && !download_config.allow_download {
            self.set_error(
        "OCR models missing and downloads are disabled. Set ATHENA_OCR_DETECTION_MODEL, ATHENA_OCR_RECOGNITION_MODEL, ATHENA_OCR_DICT_PATH, or unset ATHENA_OCR_DISABLE_DOWNLOAD.",
      );
            return;
        }
        self.ocr_rx = Some(rx);
        self.ocr_started_at = Some(Instant::now());
        let status = if missing_models {
            "Downloading OCR models..."
        } else {
            "Running OCR..."
        };
        self.set_state(UiState::Processing, status);

        thread::spawn(move || {
            let result = ensure_models(&paths, &download_config)
                .and_then(|_| OarOcrEngine::from_paths(&paths))
                .and_then(|mut engine| engine.extract_text(&OcrInput::Bytes(bytes)));
            let message = match result {
                Ok(OcrResult { text, .. }) => OcrResponse::Success(text),
                Err(error) => OcrResponse::Error(error.to_string()),
            };
            let _ = tx.send(message);
        });
    }

    /// Polls the OCR/PDF worker channel and applies results to the UI state.
    fn poll_ocr(&mut self, ctx: &egui::Context) {
        let Some(receiver) = self.ocr_rx.as_ref() else {
            return;
        };

        match receiver.try_recv() {
            Ok(message) => {
                self.ocr_rx = None;
                self.ocr_started_at = None;
                match message {
                    OcrResponse::Success(text) => {
                        self.ocr_text = text.trim().to_string();
                        if self.ocr_text.is_empty() {
                            self.set_error("No text detected. Try another file.");
                        } else {
                            self.set_state(UiState::Preview, "Text ready.");
                            self.start_session();
                        }
                    }
                    OcrResponse::Error(error) => {
                        self.set_error(format!("Processing failed: {error}"))
                    }
                }
            }
            Err(TryRecvError::Empty) => {
                if self.ui_state == UiState::Processing {
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
            }
            Err(TryRecvError::Disconnected) => {
                self.ocr_rx = None;
                self.ocr_started_at = None;
                self.set_error("OCR worker disconnected.");
            }
        }
    }

    /// Returns a human-friendly processing timer string for the status overlay.
    fn ocr_progress(&self) -> Option<String> {
        if self.ui_state != UiState::Processing {
            return None;
        }
        self.ocr_started_at.map(|start| {
            let elapsed = start.elapsed().as_secs_f32();
            format!("Processing ({elapsed:.1}s)")
        })
    }

    /// Reads text or an image from the clipboard and starts reading/OCR.
    fn paste_from_clipboard(&mut self) {
        if self.ui_state == UiState::Processing {
            self.set_status("Processing already running.");
            return;
        }

        let mut clipboard = match Clipboard::new() {
            Ok(clipboard) => clipboard,
            Err(error) => {
                self.set_error(format!("Clipboard error: {error}"));
                return;
            }
        };

        if let Ok(text) = clipboard.get_text() {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                self.session = None;
                self.next_tick = None;
                self.ocr_text = trimmed.to_string();
                self.set_state(UiState::Preview, "Text ready.");
                self.start_session();
                return;
            }
        }

        let image = match clipboard.get_image() {
            Ok(image) => image,
            Err(error) => {
                self.set_error(format!("No text or image in clipboard: {error}"));
                return;
            }
        };

        let buffer = match ImageBuffer::<Rgba<u8>, _>::from_raw(
            image.width as u32,
            image.height as u32,
            image.bytes.into_owned(),
        ) {
            Some(buffer) => buffer,
            None => {
                self.set_error("Clipboard image buffer was invalid.");
                return;
            }
        };

        let dynamic = DynamicImage::ImageRgba8(buffer);
        let mut png_bytes = Vec::new();
        if let Err(error) =
            dynamic.write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png)
        {
            self.set_error(format!("Failed to encode image: {error}"));
            return;
        }

        self.session = None;
        self.next_tick = None;
        self.ocr_text.clear();
        self.spawn_ocr(png_bytes);
    }

    /// Polls the non-blocking Import dialog worker and starts processing when a file is chosen.
    fn poll_import(&mut self, ctx: &egui::Context) {
        let Some(receiver) = self.import_rx.as_ref() else {
            return;
        };

        // Keep pumping frames even while the OS-native dialog has focus.
        ctx.request_repaint_after(Duration::from_millis(100));

        match receiver.try_recv() {
            Ok(message) => {
                self.import_rx = None;
                match message {
                    ImportResponse::Canceled => self.set_status("Import canceled."),
                    ImportResponse::Selected {
                        bytes,
                        is_pdf,
                        is_txt,
                        is_epub,
                    } => {
                        self.session = None;
                        self.next_tick = None;
                        self.ocr_text.clear();
                        if is_txt {
                            self.ocr_text = String::from_utf8_lossy(&bytes).trim().to_string();
                            if self.ocr_text.is_empty() {
                                self.set_error("No text detected. Try another file.");
                            } else {
                                self.set_state(UiState::Preview, "Text ready.");
                                self.start_session();
                            }
                        } else if is_epub {
                            self.spawn_epub_text_extract(bytes);
                        } else if is_pdf {
                            self.spawn_pdf_text_extract(bytes);
                        } else {
                            self.spawn_ocr(bytes);
                        }
                    }
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.import_rx = None;
                self.set_error("Import worker disconnected.");
            }
        }
    }

    /// Spawns a background worker to extract embedded text from a PDF file.
    fn spawn_pdf_text_extract(&mut self, bytes: Vec<u8>) {
        if self.ui_state == UiState::Processing {
            self.set_status("Processing already running.");
            return;
        }

        let (tx, rx) = mpsc::channel();
        self.ocr_rx = Some(rx);
        self.ocr_started_at = Some(Instant::now());
        self.set_state(UiState::Processing, "Extracting PDF text...");

        thread::spawn(move || {
            let result = pdf_extract::extract_text_from_mem(&bytes)
                .map_err(|error| error.to_string())
                .map(|text| text.trim().to_string());
            let message = match result {
                Ok(text) => OcrResponse::Success(text),
                Err(error) => OcrResponse::Error(error),
            };
            let _ = tx.send(message);
        });
    }

    /// Spawns a background worker to extract plain text from an EPUB file.
    fn spawn_epub_text_extract(&mut self, bytes: Vec<u8>) {
        if self.ui_state == UiState::Processing {
            self.set_status("Processing already running.");
            return;
        }

        let (tx, rx) = mpsc::channel();
        self.ocr_rx = Some(rx);
        self.ocr_started_at = Some(Instant::now());
        self.set_state(UiState::Processing, "Extracting EPUB text...");

        thread::spawn(move || {
            let result = athena_core::epub::extract_text_from_bytes(&bytes);
            let message = match result {
                Ok(text) => OcrResponse::Success(text),
                Err(error) => OcrResponse::Error(error),
            };
            let _ = tx.send(message);
        });
    }

    /// Opens an OS file picker (non-blocking) and starts OCR or PDF extraction based on selection.
    fn import_image_from_file(&mut self) {
        if self.ui_state == UiState::Processing || self.import_rx.is_some() {
            self.set_status("Processing already running.");
            return;
        }

        let (tx, rx) = mpsc::channel();
        self.import_rx = Some(rx);
        self.set_status("Waiting for file selection...");

        thread::spawn(move || {
            let picked = pollster::block_on(
                rfd::AsyncFileDialog::new()
                    .add_filter(
                        "Image",
                        &["png", "jpg", "jpeg", "bmp", "tif", "tiff", "webp"],
                    )
                    .add_filter("PDF", &["pdf"])
                    .add_filter("EPUB", &["epub"])
                    .add_filter("Text", &["txt"])
                    .pick_file(),
            );

            let message = match picked {
                None => ImportResponse::Canceled,
                Some(file) => {
                    let is_pdf = file
                        .path()
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"));
                    let is_txt = file
                        .path()
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"));
                    let is_epub = file
                        .path()
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("epub"));
                    let bytes = pollster::block_on(file.read());
                    ImportResponse::Selected {
                        bytes,
                        is_pdf,
                        is_txt,
                        is_epub,
                    }
                }
            };

            let _ = tx.send(message);
        });
    }

    /// Toggle playback, starting or pausing the reading session as needed.
    fn toggle_playback(&mut self) {
        if let Some(session) = self.session.as_ref() {
            if session.is_playing {
                self.pause();
            } else {
                self.play();
            }
        } else {
            self.play();
        }
    }

    /// Handle global keyboard shortcuts while avoiding focused text inputs.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }

        let mut paste = false;
        let mut toggle_play = false;
        let mut step_back = false;
        let mut step_forward = false;
        let mut restart = false;

        ctx.input_mut(|input| {
            paste |= input.consume_key(egui::Modifiers::COMMAND, egui::Key::V);
            toggle_play |= input.consume_key(egui::Modifiers::NONE, egui::Key::Space);
            step_back |= input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft);
            step_forward |= input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight);
            restart |= input.consume_key(egui::Modifiers::NONE, egui::Key::R);
        });

        if paste {
            self.paste_from_clipboard();
        }
        if toggle_play {
            self.toggle_playback();
        }
        if step_back {
            self.rewind(5);
        }
        if step_forward {
            self.advance(5);
        }
        if restart {
            self.restart();
        }
    }

    fn start_session(&mut self) {
        let tokens = text::normalize_and_tokenize(&self.ocr_text);
        if tokens.is_empty() {
            self.session = None;
            self.next_tick = None;
            self.edit_anchor = None;
            self.clear_paused_reading_cache();
            self.set_error("No readable tokens found in text.");
            return;
        }
        let mut session = ReadingSession::new(self.ocr_text.clone(), tokens, self.settings.wpm);
        session.set_chunk_size(self.settings.chunk_size);
        session.set_playing(false);
        self.session = Some(session);
        self.next_tick = None;
        self.edit_anchor = None;
        self.set_state(UiState::Reading, "Ready.");
        self.persist_paused_reading_cache();
    }

    fn open_editor_from_preview(&mut self) {
        self.edit_anchor = self
            .session
            .as_ref()
            .and_then(|session| build_edit_anchor(&session.tokens, session.current_index));

        if self
            .session
            .as_ref()
            .is_some_and(|session| session.is_playing)
        {
            self.pause();
        }

        self.ocr_editor_draft = self.ocr_text.clone();
        self.ocr_editor_open = true;
    }

    fn apply_editor_save(&mut self) {
        self.ocr_text = self.ocr_editor_draft.trim().to_string();
        let tokens = text::normalize_and_tokenize(&self.ocr_text);
        if tokens.is_empty() {
            self.session = None;
            self.next_tick = None;
            self.edit_anchor = None;
            self.clear_paused_reading_cache();
            self.set_error("No readable tokens found in text.");
            return;
        }

        let mapped_index = self
            .edit_anchor
            .as_ref()
            .map(|anchor| remap_index_after_edit(anchor, &tokens))
            .unwrap_or(0);

        let mut session = ReadingSession::new(self.ocr_text.clone(), tokens, self.settings.wpm);
        session.set_chunk_size(self.settings.chunk_size);
        session.set_playing(false);
        session.current_index = mapped_index.min(session.tokens.len().saturating_sub(1));

        self.session = Some(session);
        self.next_tick = None;
        self.edit_anchor = None;
        self.set_state(UiState::Reading, "Ready.");
        self.persist_paused_reading_cache();
    }

    fn play(&mut self) {
        if let Some(session) = self.session.as_mut() {
            session.set_playing(true);
            self.next_tick = None;
            self.set_state(UiState::Reading, "Playing.");
        } else {
            self.set_status("Create a session before playing.");
        }
    }

    fn pause(&mut self) {
        let mut paused = false;
        if let Some(session) = self.session.as_mut() {
            session.set_playing(false);
            self.next_tick = None;
            self.set_state(UiState::Reading, "Paused.");
            paused = true;
        }
        if paused {
            self.persist_paused_reading_cache();
        }
    }

    fn advance(&mut self, count: usize) {
        let mut persist_after = false;
        if let Some(session) = self.session.as_mut() {
            session.advance(count);
            persist_after = !session.is_playing;
        }
        if persist_after {
            self.persist_paused_reading_cache();
        }
    }

    fn rewind(&mut self, count: usize) {
        let mut persist_after = false;
        if let Some(session) = self.session.as_mut() {
            session.rewind(count);
            persist_after = !session.is_playing;
        }
        if persist_after {
            self.persist_paused_reading_cache();
        }
    }

    fn restart(&mut self) {
        let mut restarted = false;
        if let Some(session) = self.session.as_mut() {
            session.restart();
            session.set_playing(false);
            self.next_tick = None;
            self.set_state(UiState::Reading, "Restarted.");
            restarted = true;
        }
        if restarted {
            self.persist_paused_reading_cache();
        }
    }

    fn tick(&mut self, ctx: &egui::Context) {
        let mut wpm_error = false;
        let mut interval = None;

        if let Some(session) = self.session.as_mut() {
            if !session.is_playing {
                self.next_tick = None;
                return;
            }

            if let Some(next_interval) = interval_ms(session.wpm).map(Duration::from_millis) {
                interval = Some(next_interval);
                let now = Instant::now();
                let next_tick = self.next_tick.get_or_insert_with(|| now + next_interval);

                if now >= *next_tick {
                    session.advance(session.chunk_size);
                    *next_tick = now + next_interval;
                }
            } else {
                session.set_playing(false);
                self.next_tick = None;
                wpm_error = true;
            }
        }

        if wpm_error {
            self.set_error("WPM must be greater than zero.");
            return;
        }

        if let Some(interval) = interval {
            ctx.request_repaint_after(interval);
        }
    }

    fn orp_split(text: &str) -> Option<(&str, &str, &str)> {
        if text.is_empty() {
            return None;
        }

        let center_char_index = text.chars().count() / 2;
        for (current, (byte_start, ch)) in text.char_indices().enumerate() {
            if current == center_char_index {
                let byte_end = byte_start + ch.len_utf8();
                return Some((
                    &text[..byte_start],
                    &text[byte_start..byte_end],
                    &text[byte_end..],
                ));
            }
        }

        // Fallback for non-empty strings if iteration fails unexpectedly.
        let mut chars = text.char_indices();
        let (_, first) = chars.next()?;
        let first_len = first.len_utf8();
        Some(("", &text[..first_len], &text[first_len..]))
    }

    fn paint_orp_text(&self, ui: &mut egui::Ui, rect: egui::Rect, text: &str) {
        let Some((left, center, right)) = Self::orp_split(text) else {
            return;
        };

        let font_id = egui::FontId::proportional(self.settings.font_size as f32);
        let normal_color = ui.visuals().text_color();
        let highlight_color = match self.settings.theme {
            Theme::Light => egui::Color32::from_rgb(0, 120, 255),
            _ => egui::Color32::YELLOW,
        };

        let (left_galley, center_galley, right_galley) = ui.ctx().fonts_mut(|fonts| {
            (
                fonts.layout_no_wrap(left.to_string(), font_id.clone(), normal_color),
                fonts.layout_no_wrap(center.to_string(), font_id.clone(), highlight_color),
                fonts.layout_no_wrap(right.to_string(), font_id.clone(), normal_color),
            )
        });

        let center_x = rect.center().x;
        let center_y = rect.center().y;

        let center_pos_x = center_x - center_galley.size().x / 2.0;
        let baseline_y = center_y - center_galley.size().y / 2.0;

        let left_pos = egui::pos2(center_pos_x - left_galley.size().x, baseline_y);
        let center_pos = egui::pos2(center_pos_x, baseline_y);
        let right_pos = egui::pos2(center_pos_x + center_galley.size().x, baseline_y);

        let painter = ui.painter_at(rect);
        painter.galley(left_pos, left_galley, normal_color);
        painter.galley(center_pos, center_galley, highlight_color);
        painter.galley(right_pos, right_galley, normal_color);
    }

    fn truncate_words(text: &str, limit: usize) -> String {
        let mut words = text.split_whitespace();
        let mut taken: Vec<&str> = Vec::with_capacity(limit);
        for _ in 0..limit {
            let Some(word) = words.next() else {
                break;
            };
            taken.push(word);
        }
        let truncated = words.next().is_some();
        let mut output = taken.join(" ");
        if truncated && !output.is_empty() {
            output.push_str(" ...");
        }
        output
    }

    /// Renders the detached editor viewport UI and returns any user action.
    fn render_ocr_editor(&mut self, ctx: &egui::Context) -> EditorAction {
        let mut action = EditorAction::default();

        // If the user closes the editor viewport via the window manager, hide it next frame.
        if ctx.input(|input| input.viewport().close_requested()) {
            action.cancel = true;
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            let buttons_height = ui.spacing().interact_size.y;
            let editor_height = (ui.available_height() - buttons_height - 12.0).max(120.0);

            egui::ScrollArea::vertical()
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                .max_height(editor_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_sized(
                        egui::vec2(ui.available_width(), editor_height),
                        egui::TextEdit::multiline(&mut self.ocr_editor_draft)
                            .desired_width(f32::INFINITY)
                            .desired_rows(24)
                            .code_editor(),
                    );
                });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                action.save |= ui.button("Save").clicked();
                action.cancel |= ui.button("Cancel").clicked();
            });
        });

        action
    }

    /// Opens the Live View window and pauses playback if currently playing.
    fn open_live_view(&mut self) {
        if self
            .session
            .as_ref()
            .is_some_and(|session| session.is_playing)
        {
            self.pause();
        }
        // Reset close flag and seed font size from app settings.
        if let Ok(mut shared) = self.live_view_shared.lock() {
            shared.close_requested = false;
            shared.font_size = self.settings.font_size as f32;
        }
        self.live_view_open = true;
    }

    /// Updates the shared Live View state from the current session and settings.
    ///
    /// Called every frame while the Live View is open. Rebuilds the pre-computed
    /// text data only when the underlying tokens change (detected by comparing
    /// `raw_text`). The per-frame cost is O(1) for highlight/settings updates.
    fn sync_live_view_shared(&mut self) {
        let Ok(mut shared) = self.live_view_shared.lock() else {
            return;
        };

        if let Some(session) = self.session.as_ref() {
            // Rebuild text data only when the token source changes.
            if self.live_view_text_source != session.raw_text {
                self.live_view_text_source = session.raw_text.clone();
                shared.text_data = Some(Arc::new(build_live_view_text_data(&session.tokens)));
            }

            shared.current_index = session.current_index;
            shared.chunk_size = session.chunk_size;
            shared.theme = self.settings.theme.clone();
            // font_size is controlled by the Live View slider, not overwritten here.
        } else {
            shared.text_data = None;
        }
    }
}

impl eframe::App for AthenaApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let editor_viewport_id = egui::ViewportId::from_hash_of("ocr_editor_viewport");
        let live_view_viewport_id = egui::ViewportId::from_hash_of("live_view_viewport");

        // If the root viewport is closing, ensure child viewports close too.
        if ctx.input(|input| input.viewport().close_requested()) {
            self.ocr_editor_open = false;
            self.live_view_open = false;
            ctx.send_viewport_cmd_to(editor_viewport_id, egui::ViewportCommand::Close);
            ctx.send_viewport_cmd_to(live_view_viewport_id, egui::ViewportCommand::Close);
        }

        self.tick(ctx);
        self.handle_shortcuts(ctx);
        self.poll_import(ctx);
        self.poll_ocr(ctx);

        let can_paste = self.ui_state != UiState::Processing;
        let is_playing = self
            .session
            .as_ref()
            .map(|session| session.is_playing)
            .unwrap_or(false);
        let has_session = self.session.is_some();
        let can_play = has_session && !is_playing;
        let can_pause = has_session && is_playing;
        let can_nav = has_session;
        let can_restart = has_session;
        let word_progress = self.session.as_ref().and_then(|session| {
            let (index, total) = session.progress();
            (total > 0).then(|| format!("{index}/{total}"))
        });

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(can_paste, egui::Button::new("Import"))
                    .clicked()
                {
                    self.import_image_from_file();
                }
                if ui
                    .add_enabled(can_paste, egui::Button::new("Paste"))
                    .clicked()
                {
                    self.paste_from_clipboard();
                }
                if ui
                    .add_enabled(can_play, egui::Button::new("Play"))
                    .clicked()
                {
                    self.play();
                }
                if ui
                    .add_enabled(can_pause, egui::Button::new("Pause"))
                    .clicked()
                {
                    self.pause();
                }
                if ui.add_enabled(can_nav, egui::Button::new("Prev")).clicked() {
                    self.rewind(5);
                }
                if ui.add_enabled(can_nav, egui::Button::new("Next")).clicked() {
                    self.advance(5);
                }
                if ui
                    .add_enabled(can_restart, egui::Button::new("Restart"))
                    .clicked()
                {
                    self.restart();
                }
            });

            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.label("WPM");
                let wpm_changed = ui
                    .add(egui::Slider::new(&mut self.settings.wpm, 100..=900))
                    .changed();

                ui.label("Chunk");
                let chunk_changed = ui
                    .add(egui::Slider::new(&mut self.settings.chunk_size, 1..=5))
                    .changed();

                ui.label("Font");
                let font_changed = ui
                    .add(egui::Slider::new(&mut self.settings.font_size, 18..=200))
                    .changed();

                ui.label("Theme");
                let theme_changed = egui::ComboBox::from_id_salt("theme_combo")
                    .selected_text(match self.settings.theme {
                        Theme::Light => "Light",
                        Theme::Dark => "Dark",
                        Theme::HighContrast => "High Contrast",
                    })
                    .show_ui(ui, |ui| {
                        let mut changed = false;
                        changed |= ui
                            .selectable_value(&mut self.settings.theme, Theme::Light, "Light")
                            .changed();
                        changed |= ui
                            .selectable_value(&mut self.settings.theme, Theme::Dark, "Dark")
                            .changed();
                        changed |= ui
                            .selectable_value(
                                &mut self.settings.theme,
                                Theme::HighContrast,
                                "High Contrast",
                            )
                            .changed();
                        changed
                    })
                    .inner
                    .unwrap_or(false);

                if wpm_changed && let Some(session) = self.session.as_mut() {
                    session.set_wpm(self.settings.wpm);
                    self.next_tick = None;
                }
                if chunk_changed && let Some(session) = self.session.as_mut() {
                    session.set_chunk_size(self.settings.chunk_size);
                }
                if font_changed || wpm_changed || chunk_changed {
                    self.persist_settings();
                }
                if theme_changed {
                    apply_theme(ctx, &self.settings.theme);
                    self.persist_settings();
                }
            });
            ui.add_space(8.0);
        });

        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Preview:");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if has_session
                            && ui
                                .add(egui::Button::new("+").min_size(egui::vec2(24.0, 24.0)))
                                .on_hover_text("Open Live View")
                                .clicked()
                        {
                            self.open_live_view();
                        }
                        if let Some(progress) = word_progress.as_deref() {
                            ui.label(progress);
                        }
                    });
                });
                let mut preview = Self::truncate_words(&self.ocr_text, OCR_PREVIEW_WORD_LIMIT);
                let response = ui.add(
                    egui::TextEdit::multiline(&mut preview)
                        .desired_rows(6)
                        .interactive(false),
                );
                let click_response = ui.interact(
                    response.rect,
                    ui.make_persistent_id("ocr_preview_click"),
                    egui::Sense::click(),
                );
                if click_response.double_clicked() {
                    self.open_editor_from_preview();
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let available = ui.available_rect_before_wrap();
            let displayed = self
                .session
                .as_ref()
                .map(|session| session.current_chunk().join(" "));

            if let Some(text) = displayed.as_deref() {
                self.paint_orp_text(ui, available, text);
            }
        });

        egui::Area::new("status_area".into())
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-10.0, -10.0))
            .interactable(false)
            .show(ctx, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                    ui.add(egui::Label::new(&self.status).extend());
                    if let Some(progress) = self.ocr_progress() {
                        ui.add(egui::Label::new(progress).extend());
                    }
                    if self.ui_state == UiState::Processing {
                        ui.add(egui::Spinner::new());
                    }
                });
            });

        if self.ocr_editor_open {
            let action = ctx.show_viewport_immediate(
                editor_viewport_id,
                egui::ViewportBuilder::default()
                    .with_title("Edit Text")
                    .with_inner_size(egui::vec2(720.0, 480.0))
                    .with_min_inner_size(egui::vec2(520.0, 320.0))
                    .with_resizable(true),
                |ctx, class| match class {
                    egui::ViewportClass::Embedded => {
                        // Fallback: embed the editor window in the parent viewport.
                        let mut action = EditorAction::default();
                        egui::Window::new("Edit Text")
                            .open(&mut self.ocr_editor_open)
                            .resizable(true)
                            .default_size(egui::vec2(720.0, 480.0))
                            .min_size(egui::vec2(520.0, 320.0))
                            .show(ctx, |ui| {
                                let buttons_height = ui.spacing().interact_size.y;
                                let editor_height =
                                    (ui.available_height() - buttons_height - 12.0).max(120.0);
                                egui::ScrollArea::vertical()
                                    .scroll_bar_visibility(
                                        egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                                    )
                                    .max_height(editor_height)
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        ui.add_sized(
                                            egui::vec2(ui.available_width(), editor_height),
                                            egui::TextEdit::multiline(&mut self.ocr_editor_draft)
                                                .desired_width(f32::INFINITY)
                                                .desired_rows(24)
                                                .code_editor(),
                                        );
                                    });

                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    action.save |= ui.button("Save").clicked();
                                    action.cancel |= ui.button("Cancel").clicked();
                                });
                            });
                        action
                    }
                    _ => self.render_ocr_editor(ctx),
                },
            );

            if action.cancel {
                self.ocr_editor_open = false;
                self.edit_anchor = None;
                ctx.send_viewport_cmd_to(editor_viewport_id, egui::ViewportCommand::Close);
            }
            if action.save {
                self.apply_editor_save();
                self.ocr_editor_open = false;
                ctx.send_viewport_cmd_to(editor_viewport_id, egui::ViewportCommand::Close);
            }
        }

        if self.live_view_open {
            // Check if the deferred callback signaled a close.
            if self
                .live_view_shared
                .lock()
                .is_ok_and(|s| s.close_requested)
            {
                self.live_view_open = false;
                ctx.send_viewport_cmd_to(live_view_viewport_id, egui::ViewportCommand::Close);
            }
        }

        if self.live_view_open {
            self.sync_live_view_shared();

            let shared = Arc::clone(&self.live_view_shared);
            ctx.show_viewport_deferred(
                live_view_viewport_id,
                egui::ViewportBuilder::default()
                    .with_title("Live View")
                    .with_inner_size(egui::vec2(720.0, 540.0))
                    .with_min_inner_size(egui::vec2(400.0, 300.0))
                    .with_resizable(true),
                move |ctx, _class| {
                    render_live_view_deferred(ctx, &shared);
                },
            );
        }
    }
}

fn build_edit_anchor(tokens: &[String], current_index: usize) -> Option<EditAnchor> {
    let token = tokens.get(current_index)?.clone();
    let token_occurrence = tokens
        .iter()
        .take(current_index + 1)
        .filter(|item| item.as_str() == token.as_str())
        .count();
    let prev_token = current_index
        .checked_sub(1)
        .and_then(|index| tokens.get(index))
        .cloned();
    let next_token = tokens.get(current_index + 1).cloned();
    Some(EditAnchor {
        token,
        token_occurrence,
        old_index: current_index,
        prev_token,
        next_token,
    })
}

fn remap_index_after_edit(anchor: &EditAnchor, new_tokens: &[String]) -> usize {
    if new_tokens.is_empty() {
        return 0;
    }

    let same_index_candidate = (anchor.old_index < new_tokens.len()).then_some(anchor.old_index);
    let anchor_token_candidate =
        find_token_occurrence(new_tokens, &anchor.token, anchor.token_occurrence);

    match (same_index_candidate, anchor_token_candidate) {
        (Some(same), Some(anchor_match)) if same == anchor_match => same,
        (Some(same), Some(anchor_match)) => {
            let same_score = remap_context_score(anchor, new_tokens, same);
            let anchor_score = remap_context_score(anchor, new_tokens, anchor_match);
            if anchor_score > same_score {
                anchor_match
            } else {
                same
            }
        }
        (Some(same), None) => same,
        (None, Some(anchor_match)) => anchor_match,
        (None, None) => 0,
    }
}

fn find_token_occurrence(
    tokens: &[String],
    needle: &str,
    target_occurrence: usize,
) -> Option<usize> {
    if target_occurrence == 0 {
        return None;
    }

    let mut seen = 0;
    for (index, token) in tokens.iter().enumerate() {
        if token == needle {
            seen += 1;
            if seen == target_occurrence {
                return Some(index);
            }
        }
    }

    None
}

fn remap_context_score(anchor: &EditAnchor, tokens: &[String], index: usize) -> usize {
    let prev_matches = anchor.prev_token.as_deref()
        == index
            .checked_sub(1)
            .and_then(|idx| tokens.get(idx))
            .map(String::as_str);
    let next_matches = anchor.next_token.as_deref() == tokens.get(index + 1).map(String::as_str);
    usize::from(prev_matches) + usize::from(next_matches)
}

fn build_restored_session(
    text_input: &str,
    settings: &UserSettings,
    current_index: usize,
) -> Option<(String, ReadingSession)> {
    let restored_text = text_input.trim().to_string();
    if restored_text.is_empty() {
        return None;
    }

    let tokens = text::normalize_and_tokenize(&restored_text);
    if tokens.is_empty() {
        return None;
    }

    let mut session = ReadingSession::new(restored_text.clone(), tokens, settings.wpm);
    session.set_chunk_size(settings.chunk_size);
    session.set_playing(false);
    session.current_index = if current_index < session.tokens.len() {
        current_index
    } else {
        0
    };
    Some((restored_text, session))
}

/// Returns the per-user configuration path for saved settings.
fn settings_path() -> Option<PathBuf> {
    ProjectDirs::from("com", "athena", "reader").map(|dirs| dirs.config_dir().join("settings.json"))
}

/// Returns the persisted paused-reading cache path.
fn reading_cache_path() -> Option<PathBuf> {
    ProjectDirs::from("com", "athena", "reader")
        .map(|dirs| dirs.config_dir().join("reading_cache.json"))
}

/// Returns the cache directory used for OCR model storage.
fn model_cache_dir() -> Option<PathBuf> {
    if let Ok(path) = env::var(ENV_OCR_CACHE_DIR)
        && !path.trim().is_empty()
    {
        return Some(PathBuf::from(path));
    }
    ProjectDirs::from("com", "athena", "reader").map(|dirs| dirs.cache_dir().join("ocrs"))
}

/// Parses a boolean environment flag based on common truthy strings.
fn env_var_truthy(name: &str) -> bool {
    env::var(name).is_ok_and(|value| {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

/// Builds the OCR model download configuration using environment overrides.
fn model_download_config() -> OcrModelDownloadConfig {
    let detection_url =
        env::var(ENV_OCR_DETECTION_URL).unwrap_or_else(|_| DEFAULT_DETECTION_URL.into());
    let recognition_url =
        env::var(ENV_OCR_RECOGNITION_URL).unwrap_or_else(|_| DEFAULT_RECOGNITION_URL.into());
    let dict_url = env::var(ENV_OCR_DICT_URL).unwrap_or_else(|_| DEFAULT_DICT_URL.into());
    let detection_sha256 = env::var(ENV_OCR_DETECTION_SHA256)
        .ok()
        .filter(|value| !value.trim().is_empty());
    let recognition_sha256 = env::var(ENV_OCR_RECOGNITION_SHA256)
        .ok()
        .filter(|value| !value.trim().is_empty());
    let dict_sha256 = env::var(ENV_OCR_DICT_SHA256)
        .ok()
        .filter(|value| !value.trim().is_empty());
    let allow_download = !env_var_truthy(ENV_OCR_DISABLE_DOWNLOAD);

    OcrModelDownloadConfig::new(
        OcrModelDownloadInfo::new(detection_url, detection_sha256),
        OcrModelDownloadInfo::new(recognition_url, recognition_sha256),
        OcrModelDownloadInfo::new(dict_url, dict_sha256),
        allow_download,
    )
}

/// Resolves OCR model file paths from environment overrides and cache defaults.
fn default_model_paths() -> Option<OcrModelPaths> {
    let detection_env = env::var(ENV_OCR_DETECTION_MODEL)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);
    let recognition_env = env::var(ENV_OCR_RECOGNITION_MODEL)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);
    let dict_env = env::var(ENV_OCR_DICT_PATH)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);
    let cache_dir = model_cache_dir();

    let detection = detection_env.or_else(|| {
        cache_dir
            .as_ref()
            .map(|dir| dir.join(DEFAULT_DETECTION_FILENAME))
    })?;
    let recognition = recognition_env.or_else(|| {
        cache_dir
            .as_ref()
            .map(|dir| dir.join(DEFAULT_RECOGNITION_FILENAME))
    })?;
    let dict = dict_env.or_else(|| {
        cache_dir
            .as_ref()
            .map(|dir| dir.join(DEFAULT_DICT_FILENAME))
    })?;

    Some(OcrModelPaths::new(detection, recognition, dict))
}

/// Applies egui visuals based on the user-selected theme.
fn apply_theme(ctx: &egui::Context, theme: &Theme) {
    match theme {
        Theme::Light => {
            let mut visuals = egui::Visuals::light();
            visuals.override_text_color = Some(egui::Color32::BLACK);
            ctx.set_visuals(visuals);
        }
        Theme::Dark => ctx.set_visuals(egui::Visuals::dark()),
        Theme::HighContrast => {
            let mut visuals = egui::Visuals::dark();
            visuals.override_text_color = Some(egui::Color32::WHITE);
            visuals.widgets.noninteractive.bg_fill = egui::Color32::BLACK;
            visuals.widgets.inactive.bg_fill = egui::Color32::BLACK;
            ctx.set_visuals(visuals);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{build_edit_anchor, build_restored_session, remap_index_after_edit};
    use athena_core::settings::UserSettings;

    fn tokens(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn remap_tracks_word_when_prefix_word_removed() {
        let old_tokens = tokens(&["one", "two", "three"]);
        let anchor = build_edit_anchor(&old_tokens, 1).expect("anchor should exist");
        let new_tokens = tokens(&["two", "three"]);
        assert_eq!(remap_index_after_edit(&anchor, &new_tokens), 0);
    }

    #[test]
    fn remap_keeps_same_index_when_anchor_missing_but_index_valid() {
        let old_tokens = tokens(&["alpha", "beta", "gamma"]);
        let anchor = build_edit_anchor(&old_tokens, 2).expect("anchor should exist");
        let new_tokens = tokens(&["one", "two", "three", "four"]);
        assert_eq!(remap_index_after_edit(&anchor, &new_tokens), 2);
    }

    #[test]
    fn remap_resets_to_start_when_anchor_missing_and_index_out_of_range() {
        let old_tokens = tokens(&["alpha", "beta", "gamma"]);
        let anchor = build_edit_anchor(&old_tokens, 2).expect("anchor should exist");
        let new_tokens = tokens(&["one"]);
        assert_eq!(remap_index_after_edit(&anchor, &new_tokens), 0);
    }

    #[test]
    fn remap_uses_nth_occurrence_for_duplicate_tokens() {
        let old_tokens = tokens(&["a", "b", "a", "c"]);
        let anchor = build_edit_anchor(&old_tokens, 2).expect("anchor should exist");
        let new_tokens = tokens(&["a", "a", "c"]);
        assert_eq!(remap_index_after_edit(&anchor, &new_tokens), 1);
    }

    #[test]
    fn remap_prefers_same_position_when_current_word_is_edited() {
        let old_tokens = tokens(&["alpha", "beta", "gamma", "alpha"]);
        let anchor = build_edit_anchor(&old_tokens, 0).expect("anchor should exist");
        let new_tokens = tokens(&["delta", "beta", "gamma", "alpha"]);
        assert_eq!(remap_index_after_edit(&anchor, &new_tokens), 0);
    }

    #[test]
    fn restore_session_keeps_valid_index() {
        let settings = UserSettings::default();
        let (_, session) =
            build_restored_session("one two three", &settings, 2).expect("should restore");
        assert_eq!(session.current_index, 2);
        assert!(!session.is_playing);
    }

    #[test]
    fn restore_session_resets_out_of_range_index_to_start() {
        let settings = UserSettings::default();
        let (_, session) =
            build_restored_session("one two", &settings, 10).expect("should restore");
        assert_eq!(session.current_index, 0);
        assert!(!session.is_playing);
    }

    // ── Live View: build_highlighted_layout tests ───────────────────────

    use super::compute_scroll_target;

    /// Test-only helper: builds a list of `(token_text, is_highlighted)` pairs.
    fn build_highlighted_layout(
        tokens: &[String],
        current_index: usize,
        chunk_size: usize,
    ) -> Vec<(String, bool)> {
        if tokens.is_empty() {
            return Vec::new();
        }
        let highlight_end = (current_index + chunk_size).min(tokens.len());
        tokens
            .iter()
            .enumerate()
            .map(|(i, token)| {
                let highlighted = i >= current_index && i < highlight_end;
                (token.clone(), highlighted)
            })
            .collect()
    }

    #[test]
    fn highlight_layout_empty_tokens_returns_empty() {
        let result = build_highlighted_layout(&[], 0, 1);
        assert!(result.is_empty());
    }

    #[test]
    fn highlight_layout_single_token_highlighted() {
        let toks = tokens(&["hello"]);
        let result = build_highlighted_layout(&toks, 0, 1);
        assert_eq!(result, vec![("hello".to_string(), true)]);
    }

    #[test]
    fn highlight_layout_mid_stream() {
        let toks = tokens(&["one", "two", "three", "four"]);
        let result = build_highlighted_layout(&toks, 1, 1);
        assert_eq!(
            result,
            vec![
                ("one".to_string(), false),
                ("two".to_string(), true),
                ("three".to_string(), false),
                ("four".to_string(), false),
            ]
        );
    }

    #[test]
    fn highlight_layout_end_of_stream() {
        let toks = tokens(&["alpha", "beta", "gamma"]);
        let result = build_highlighted_layout(&toks, 2, 1);
        assert_eq!(
            result,
            vec![
                ("alpha".to_string(), false),
                ("beta".to_string(), false),
                ("gamma".to_string(), true),
            ]
        );
    }

    #[test]
    fn highlight_layout_chunk_size_greater_than_one() {
        let toks = tokens(&["a", "b", "c", "d", "e"]);
        let result = build_highlighted_layout(&toks, 1, 3);
        assert_eq!(
            result,
            vec![
                ("a".to_string(), false),
                ("b".to_string(), true),
                ("c".to_string(), true),
                ("d".to_string(), true),
                ("e".to_string(), false),
            ]
        );
    }

    #[test]
    fn highlight_layout_chunk_clamps_at_end() {
        let toks = tokens(&["x", "y", "z"]);
        let result = build_highlighted_layout(&toks, 2, 3);
        assert_eq!(
            result,
            vec![
                ("x".to_string(), false),
                ("y".to_string(), false),
                ("z".to_string(), true),
            ]
        );
    }

    // ── Live View: compute_scroll_target tests ──────────────────────────

    #[test]
    fn scroll_target_highlight_near_top() {
        // highlight_y = 10, viewport = 600 → target = 10 - 200 = -190 → clamped to 0
        assert_eq!(compute_scroll_target(10.0, 600.0), 0.0);
    }

    #[test]
    fn scroll_target_highlight_in_middle() {
        // highlight_y = 500, viewport = 600 → target = 500 - 200 = 300
        assert_eq!(compute_scroll_target(500.0, 600.0), 300.0);
    }

    #[test]
    fn scroll_target_highlight_near_bottom() {
        // highlight_y = 1800, viewport = 600 → target = 1800 - 200 = 1600
        assert_eq!(compute_scroll_target(1800.0, 600.0), 1600.0);
    }

    #[test]
    fn scroll_target_zero_viewport_height() {
        // Zero viewport → should not go negative; returns max(0, highlight_y)
        assert_eq!(compute_scroll_target(100.0, 0.0), 100.0);
    }
}

/// Computes the scroll offset that places the highlighted element in the top
/// one-third of the viewport.
///
/// - `highlight_y`: the Y-position of the highlighted word within the full
///   content area.
/// - `viewport_height`: the visible height of the scroll area.
///
/// Returns the scroll offset (clamped to >= 0.0) such that the highlight
/// appears at roughly the 1/3 mark from the top.
fn compute_scroll_target(highlight_y: f32, viewport_height: f32) -> f32 {
    if viewport_height <= 0.0 {
        return 0.0_f32.max(highlight_y);
    }
    let target_offset = highlight_y - viewport_height / 3.0;
    target_offset.max(0.0)
}

/// Builds the pre-computed text and offset tables from session tokens.
///
/// Called once when the session tokens change, not every frame.
fn build_live_view_text_data(tokens: &[String]) -> LiveViewTextData {
    let estimated_len: usize =
        tokens.iter().map(|t| t.len()).sum::<usize>() + tokens.len().saturating_sub(1);
    let mut full_text = String::with_capacity(estimated_len);
    let mut token_byte_starts = Vec::with_capacity(tokens.len());
    let mut token_byte_ends = Vec::with_capacity(tokens.len());
    let mut token_char_starts = Vec::with_capacity(tokens.len());
    let mut char_count = 0usize;

    for (i, token) in tokens.iter().enumerate() {
        if i > 0 {
            full_text.push(' ');
            char_count += 1;
        }
        token_byte_starts.push(full_text.len());
        token_char_starts.push(char_count);
        full_text.push_str(token);
        char_count += token.chars().count();
        token_byte_ends.push(full_text.len());
    }

    LiveViewTextData {
        full_text,
        token_byte_starts,
        token_byte_ends,
        token_char_starts,
    }
}

/// Deferred rendering callback for the Live View viewport.
///
/// Reads the shared state snapshot, builds a `LayoutJob` with 1–3 sections,
/// and renders the flowing text with the highlighted word and smooth
/// auto-scroll. Runs in the child viewport's context, decoupled from the
/// parent event loop.
fn render_live_view_deferred(ctx: &egui::Context, shared: &Arc<Mutex<LiveViewShared>>) {
    // Once close has been requested (this frame or a prior one), render only
    // a cheap empty panel for every remaining frame during the Wayland close
    // animation, avoiding expensive LayoutJob relayouts on each resize step.
    let already_closing = shared.lock().is_ok_and(|s| s.close_requested);
    if already_closing || ctx.input(|input| input.viewport().close_requested()) {
        if !already_closing && let Ok(mut state) = shared.lock() {
            state.close_requested = true;
        }
        ctx.request_repaint_of(egui::ViewportId::ROOT);
        egui::CentralPanel::default().show(ctx, |_| {});
        return;
    }

    // When minimized on Wayland, rendering blocks on compositor frame callbacks
    // that never arrive, stalling the shared event loop and freezing the parent.
    // Render a cheap empty panel and skip all repaint requests until unminimized.
    let minimized = ctx.input(|i| i.viewport().minimized == Some(true));
    if minimized {
        egui::CentralPanel::default().show(ctx, |_| {});
        return;
    }

    // Read the snapshot under a short lock, then drop it before rendering.
    let (text_data, current_index, chunk_size, theme, mut font_size, mut window_start) = {
        let Ok(state) = shared.lock() else {
            return;
        };
        let text_data = match state.text_data.as_ref() {
            Some(td) => Arc::clone(td),
            None => {
                drop(state);
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label("No reading session active.");
                });
                return;
            }
        };
        (
            text_data,
            state.current_index,
            state.chunk_size,
            state.theme.clone(),
            state.font_size,
            state.window_start,
        )
    };

    if text_data.full_text.is_empty() || text_data.token_byte_starts.is_empty() {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("No tokens to display.");
        });
        return;
    }

    // Bottom panel with the font size slider.
    egui::TopBottomPanel::bottom("live_view_controls").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Font size:");
            ui.add(egui::Slider::new(&mut font_size, 18.0..=200.0));
        });
    });

    // Write the (possibly updated) font size back.
    if let Ok(mut state) = shared.lock() {
        state.font_size = font_size;
    }

    // Compute the stable sliding window BEFORE the panel closure so we can
    // persist the updated window_start afterwards.
    let num_tokens = text_data.token_byte_starts.len();
    let current = current_index.min(num_tokens.saturating_sub(1));
    let highlight_end = (current + chunk_size).min(num_tokens);

    // window_start is pinned so text before the highlight never reflows.
    // The window only grows at the trailing edge as words stream in.
    // Reset only when the highlight escapes the current window (rewind or
    // after streaming past 2×TOKEN_WINDOW tokens from window_start).
    const TOKEN_WINDOW: usize = 1500;

    if current < window_start || current > window_start + 2 * TOKEN_WINDOW {
        window_start = current.saturating_sub(TOKEN_WINDOW);
    }
    let window_end = (window_start + 3 * TOKEN_WINDOW).min(num_tokens);

    // Persist the (possibly updated) window position.
    if let Ok(mut state) = shared.lock() {
        state.window_start = window_start;
    }

    let base_byte = text_data.token_byte_starts[window_start];
    let end_byte = text_data.token_byte_ends[window_end - 1];
    let windowed_text = text_data.full_text[base_byte..end_byte].to_owned();

    let hl_byte_start = text_data.token_byte_starts[current] - base_byte;
    let hl_byte_end = if highlight_end > 0 {
        text_data.token_byte_ends[highlight_end - 1] - base_byte
    } else {
        hl_byte_start
    };
    let hl_char_start =
        text_data.token_char_starts[current] - text_data.token_char_starts[window_start];

    egui::CentralPanel::default().show(ctx, |ui| {
        let normal_color = ui.visuals().text_color();
        let highlight_color = match theme {
            Theme::Light => egui::Color32::from_rgb(0, 120, 255),
            _ => egui::Color32::YELLOW,
        };
        let clamped_font_size = font_size.clamp(18.0, 200.0);
        let font_id = egui::FontId::proportional(clamped_font_size);

        let normal_fmt = egui::text::TextFormat {
            font_id: font_id.clone(),
            color: normal_color,
            ..Default::default()
        };
        let highlight_fmt = egui::text::TextFormat {
            font_id,
            color: highlight_color,
            underline: egui::Stroke::new(2.0, highlight_color),
            ..Default::default()
        };

        let windowed_len = windowed_text.len();
        let mut sections = Vec::with_capacity(3);
        if hl_byte_start > 0 {
            sections.push(egui::text::LayoutSection {
                leading_space: 0.0,
                byte_range: 0..hl_byte_start,
                format: normal_fmt.clone(),
            });
        }
        sections.push(egui::text::LayoutSection {
            leading_space: 0.0,
            byte_range: hl_byte_start..hl_byte_end,
            format: highlight_fmt,
        });
        if hl_byte_end < windowed_len {
            sections.push(egui::text::LayoutSection {
                leading_space: 0.0,
                byte_range: hl_byte_end..windowed_len,
                format: normal_fmt,
            });
        }

        let job = egui::text::LayoutJob {
            text: windowed_text.clone(),
            sections,
            wrap: egui::text::TextWrapping {
                max_width: ui.available_width(),
                ..Default::default()
            },
            ..Default::default()
        };

        let galley = ui.ctx().fonts_mut(|fonts| fonts.layout_job(job));

        let highlight_y = galley
            .pos_from_cursor(egui::text::CCursor::new(hl_char_start))
            .top();

        let viewport_height = ui.available_height();
        let target_offset = compute_scroll_target(highlight_y, viewport_height);

        let scroll_id = ui.id().with("live_view_scroll");
        let smoothed_offset = ctx.animate_value_with_time(scroll_id, target_offset, 0.3);

        egui::ScrollArea::vertical()
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
            .auto_shrink([false, false])
            .vertical_scroll_offset(smoothed_offset)
            .show(ui, |ui| {
                ui.label(galley);
            });
    });

    // Self-schedule a gentle poll so the child picks up shared-state changes
    // (e.g. stream advance) without the parent forcing repaints. The backend
    // handles this correctly for minimized windows, unlike request_repaint_of
    // from the parent which can stall the event loop on Wayland.
    ctx.request_repaint_after(std::time::Duration::from_millis(50));
}

/// GUI entry point.
fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([960.0, 540.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Athena Reader",
        native_options,
        Box::new(|cc| Ok(Box::new(AthenaApp::new(cc)))),
    )
}
