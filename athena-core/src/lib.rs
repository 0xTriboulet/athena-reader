//! Athena Reader core library.
//!
//! This crate contains the platform-agnostic pieces used by the GUI application:
//! - OCR engine abstraction and model management
//! - Text normalization/tokenization
//! - Reading session state and timing helpers
//! - User settings persistence types

/// OCR engine abstraction, model downloads, and image preprocessing helpers.
pub mod ocr;
/// Reading session state (current index, chunking, WPM timing).
pub mod reader;
/// User settings model + load/save helpers.
pub mod settings;
/// Text normalization/tokenization used to build a reading session.
pub mod text;
