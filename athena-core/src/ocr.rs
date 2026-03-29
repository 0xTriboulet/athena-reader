//! OCR engine abstraction and model management.
//!
//! This module wraps the OCR backend used by Athena Reader:
//! - Image preprocessing helpers
//! - ONNX model download/caching + optional integrity checks
//! - An [`OcrEngine`] trait implemented by the default `oar-ocr` backend

use std::fmt;
use std::path::PathBuf;

use oar_ocr::oarocr::{OAROCR, OAROCRBuilder};

use crate::model_utils::{self, ModelDownloadInfo, ModelError};

/// Errors returned by OCR preprocessing, configuration, model download, or engine execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OcrError {
    /// Required configuration (model paths, etc) is missing.
    NotConfigured(String),
    /// The input was not a supported image/PDF representation.
    UnsupportedInput(String),
    /// The OCR backend failed to run.
    EngineFailure(String),
}

impl fmt::Display for OcrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OcrError::NotConfigured(message) => write!(f, "OCR not configured: {message}"),
            OcrError::UnsupportedInput(message) => write!(f, "Unsupported OCR input: {message}"),
            OcrError::EngineFailure(message) => write!(f, "OCR engine failure: {message}"),
        }
    }
}

impl std::error::Error for OcrError {}

/// Input accepted by an OCR engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OcrInput {
    /// OCR should read the image from a file path.
    Path(PathBuf),
    /// OCR should read the image from an in-memory encoded image (PNG/JPEG/etc).
    Bytes(Vec<u8>),
}

/// OCR output text and optional confidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrResult {
    /// Extracted plaintext.
    pub text: String,
    /// Optional confidence score (backend-specific).
    pub confidence: Option<u32>,
}

/// RGB pixel buffer produced by [`preprocess_image_bytes`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreprocessedImage {
    /// Image width (pixels).
    pub width: u32,
    /// Image height (pixels).
    pub height: u32,
    /// RGB pixel data (3 bytes per pixel).
    pub pixels: Vec<u8>,
}

/// Decodes an encoded image and converts it to a packed RGB byte buffer.
pub fn preprocess_image_bytes(bytes: &[u8]) -> Result<PreprocessedImage, OcrError> {
    let image = image::load_from_memory(bytes)
        .map_err(|error| OcrError::UnsupportedInput(error.to_string()))?;
    let rgb = image.to_rgb8();
    let (width, height) = rgb.dimensions();
    Ok(PreprocessedImage {
        width,
        height,
        pixels: rgb.into_raw(),
    })
}

fn rgb_image_from_preprocessed(image: PreprocessedImage) -> Result<image::RgbImage, OcrError> {
    image::RgbImage::from_raw(image.width, image.height, image.pixels).ok_or_else(|| {
        OcrError::UnsupportedInput("Decoded image contained invalid RGB pixel data.".into())
    })
}

/// Minimal OCR engine interface used by the GUI.
pub trait OcrEngine {
    /// Extracts plaintext from the given input.
    fn extract_text(&mut self, input: &OcrInput) -> Result<OcrResult, OcrError>;
}

/// Paths to the OCR model artifacts required by the default backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrModelPaths {
    /// Text detection ONNX model file.
    pub detection: PathBuf,
    /// Text recognition ONNX model file.
    pub recognition: PathBuf,
    /// Character dictionary file.
    pub dict: PathBuf,
}

impl OcrModelPaths {
    /// Builds a model-path bundle from detection, recognition, and dictionary file paths.
    pub fn new(detection: PathBuf, recognition: PathBuf, dict: PathBuf) -> Self {
        Self {
            detection,
            recognition,
            dict,
        }
    }
}

/// Default filename for cached text detection models.
pub const DEFAULT_DETECTION_FILENAME: &str = "det.onnx";
/// Default filename for cached text recognition models.
pub const DEFAULT_RECOGNITION_FILENAME: &str = "rec.onnx";
/// Default filename for cached character dictionary files.
pub const DEFAULT_DICT_FILENAME: &str = "dict.txt";
/// Default download URL for the PaddleOCR detection ONNX model.
pub const DEFAULT_DETECTION_URL: &str =
    "https://huggingface.co/GetcharZp/go-ocr/resolve/main/paddle_weights/det.onnx?download=true";
/// Default download URL for the PaddleOCR recognition ONNX model.
pub const DEFAULT_RECOGNITION_URL: &str =
    "https://huggingface.co/GetcharZp/go-ocr/resolve/main/paddle_weights/rec.onnx?download=true";
/// Default download URL for the PaddleOCR character dictionary.
pub const DEFAULT_DICT_URL: &str =
    "https://huggingface.co/GetcharZp/go-ocr/resolve/main/paddle_weights/dict.txt?download=true";

/// Re-export of the shared model download descriptor.
pub type OcrModelDownloadInfo = ModelDownloadInfo;

/// Configures download behavior for the OCR detection and recognition models.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrModelDownloadConfig {
    pub detection: OcrModelDownloadInfo,
    pub recognition: OcrModelDownloadInfo,
    pub dict: OcrModelDownloadInfo,
    pub allow_download: bool,
}

impl OcrModelDownloadConfig {
    /// Creates a download configuration that governs both OCR model artifacts.
    pub fn new(
        detection: OcrModelDownloadInfo,
        recognition: OcrModelDownloadInfo,
        dict: OcrModelDownloadInfo,
        allow_download: bool,
    ) -> Self {
        Self {
            detection,
            recognition,
            dict,
            allow_download,
        }
    }
}

/// Reports whether models were downloaded during a preparation step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OcrModelAvailability {
    pub downloaded_detection: bool,
    pub downloaded_recognition: bool,
    pub downloaded_dict: bool,
}

impl OcrModelAvailability {
    /// Returns true when any model file was downloaded.
    pub fn downloaded_any(self) -> bool {
        self.downloaded_detection || self.downloaded_recognition || self.downloaded_dict
    }
}

pub struct OarOcrEngine {
    engine: OAROCR,
}

impl OarOcrEngine {
    pub fn from_paths(paths: &OcrModelPaths) -> Result<Self, OcrError> {
        if !paths.detection.exists() {
            return Err(OcrError::NotConfigured(format!(
                "Detection model not found at {}",
                paths.detection.display()
            )));
        }
        if !paths.recognition.exists() {
            return Err(OcrError::NotConfigured(format!(
                "Recognition model not found at {}",
                paths.recognition.display()
            )));
        }
        if !paths.dict.exists() {
            return Err(OcrError::NotConfigured(format!(
                "Dictionary file not found at {}",
                paths.dict.display()
            )));
        }

        let engine = OAROCRBuilder::new(
            paths.detection.clone(),
            paths.recognition.clone(),
            paths.dict.clone(),
        )
        .build()
        .map_err(|error| OcrError::EngineFailure(error.to_string()))?;

        Ok(Self { engine })
    }
}

/// Ensures OCR models exist on disk, optionally downloading and verifying them.
pub fn ensure_models(
    paths: &OcrModelPaths,
    config: &OcrModelDownloadConfig,
) -> Result<OcrModelAvailability, OcrError> {
    let map_err = |e: ModelError| match e {
        ModelError::NotConfigured(msg) => OcrError::NotConfigured(msg),
        ModelError::Failure(msg) => OcrError::EngineFailure(msg),
    };
    let downloaded_detection =
        model_utils::ensure_model_file(&paths.detection, &config.detection, config.allow_download)
            .map_err(map_err)?;
    let downloaded_recognition = model_utils::ensure_model_file(
        &paths.recognition,
        &config.recognition,
        config.allow_download,
    )
    .map_err(map_err)?;
    let downloaded_dict =
        model_utils::ensure_model_file(&paths.dict, &config.dict, config.allow_download)
            .map_err(map_err)?;
    Ok(OcrModelAvailability {
        downloaded_detection,
        downloaded_recognition,
        downloaded_dict,
    })
}

impl OcrEngine for OarOcrEngine {
    fn extract_text(&mut self, input: &OcrInput) -> Result<OcrResult, OcrError> {
        let bytes = match input {
            OcrInput::Path(path) => std::fs::read(path)
                .map_err(|error| OcrError::UnsupportedInput(error.to_string()))?,
            OcrInput::Bytes(bytes) => bytes.clone(),
        };

        let processed = preprocess_image_bytes(&bytes)?;
        let rgb = rgb_image_from_preprocessed(processed)?;
        let mut results = self
            .engine
            .predict(vec![rgb])
            .map_err(|error| OcrError::EngineFailure(error.to_string()))?;
        let result = results.pop().ok_or_else(|| {
            OcrError::EngineFailure("OCR engine returned an empty result set.".into())
        })?;
        let confidence = result
            .average_confidence()
            .map(|value| (value * 100.0).round() as u32);

        Ok(OcrResult {
            text: result.concatenated_text("\n"),
            confidence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_utils;
    use image::{DynamicImage, ImageBuffer, Rgb};

    #[test]
    fn preprocess_image_bytes_decodes_rgb_pixels() {
        let image = ImageBuffer::from_fn(2, 1, |x, _| {
            if x == 0 {
                Rgb([0u8, 10u8, 20u8])
            } else {
                Rgb([200u8, 210u8, 220u8])
            }
        });
        let mut bytes = Vec::new();
        DynamicImage::ImageRgb8(image)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();

        let processed = preprocess_image_bytes(&bytes).unwrap();
        assert_eq!(processed.width, 2);
        assert_eq!(processed.height, 1);
        assert_eq!(processed.pixels.len(), 2 * 1 * 3);
        assert_eq!(&processed.pixels[0..3], &[0, 10, 20]);
        assert_eq!(&processed.pixels[3..6], &[200, 210, 220]);
    }

    #[test]
    fn preprocess_image_bytes_rejects_invalid_input() {
        let result = preprocess_image_bytes(&[0u8, 1u8, 2u8]);
        assert!(matches!(result, Err(OcrError::UnsupportedInput(_))));
    }

    #[test]
    fn oar_ocr_engine_requires_model_paths() {
        let dir = tempfile::tempdir().unwrap();
        let paths = OcrModelPaths::new(
            dir.path().join("detector.onnx"),
            dir.path().join("recognizer.onnx"),
            dir.path().join("dict.txt"),
        );
        let result = OarOcrEngine::from_paths(&paths);
        assert!(matches!(result, Err(OcrError::NotConfigured(_))));
    }

    #[test]
    fn ensure_models_requires_download_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let paths = OcrModelPaths::new(
            dir.path().join("detector.onnx"),
            dir.path().join("recognizer.onnx"),
            dir.path().join("dict.txt"),
        );
        let config = OcrModelDownloadConfig::new(
            OcrModelDownloadInfo::new("https://example.com/detector.onnx", None),
            OcrModelDownloadInfo::new("https://example.com/recognizer.onnx", None),
            OcrModelDownloadInfo::new("https://example.com/dict.txt", None),
            false,
        );
        let result = ensure_models(&paths, &config);
        assert!(matches!(result, Err(OcrError::NotConfigured(_))));
    }

    #[test]
    fn sha256_file_verifies_integrity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.bin");
        std::fs::write(&path, b"athena").unwrap();
        let hash = model_utils::sha256_file(&path).unwrap();
        assert!(model_utils::verify_sha256(&path, &hash).unwrap());
    }
}
