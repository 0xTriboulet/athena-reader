//! OCR engine abstraction and model management.
//!
//! This module wraps the OCR backend used by Athena Reader:
//! - Image preprocessing helpers
//! - ONNX model download/caching + optional integrity checks
//! - An [`OcrEngine`] trait implemented by the default `oar-ocr` backend

use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use oar_ocr::oarocr::{OAROCR, OAROCRBuilder};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

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

/// Describes a single OCR model download source and optional integrity hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrModelDownloadInfo {
    pub url: String,
    pub sha256: Option<String>,
}

impl OcrModelDownloadInfo {
    /// Creates a download descriptor for a model file.
    pub fn new(url: impl Into<String>, sha256: Option<String>) -> Self {
        Self {
            url: url.into(),
            sha256,
        }
    }
}

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
    let downloaded_detection =
        ensure_model_file(&paths.detection, &config.detection, config.allow_download)?;
    let downloaded_recognition = ensure_model_file(
        &paths.recognition,
        &config.recognition,
        config.allow_download,
    )?;
    let downloaded_dict = ensure_model_file(&paths.dict, &config.dict, config.allow_download)?;
    Ok(OcrModelAvailability {
        downloaded_detection,
        downloaded_recognition,
        downloaded_dict,
    })
}

/// Ensures a single model file exists and passes integrity checks.
fn ensure_model_file(
    path: &Path,
    info: &OcrModelDownloadInfo,
    allow_download: bool,
) -> Result<bool, OcrError> {
    if path.exists() {
        if let Some(expected) = normalize_hash(info.sha256.as_deref()) {
            if verify_sha256(path, &expected)? {
                return Ok(false);
            }
            if !allow_download {
                return Err(OcrError::NotConfigured(format!(
                    "Model at {} failed integrity check and downloads are disabled.",
                    path.display()
                )));
            }
            let _ = fs::remove_file(path);
        } else {
            return Ok(false);
        }
    }

    if !allow_download {
        return Err(OcrError::NotConfigured(format!(
            "Model missing at {} and downloads are disabled.",
            path.display()
        )));
    }

    download_to_path(&info.url, path)?;
    if let Some(expected) = normalize_hash(info.sha256.as_deref())
        && !verify_sha256(path, &expected)?
    {
        let _ = fs::remove_file(path);
        return Err(OcrError::EngineFailure(format!(
            "Downloaded model failed integrity check at {}.",
            path.display()
        )));
    }
    Ok(true)
}

/// Downloads a model file to the given path using a temporary file.
fn download_to_path(url: &str, path: &Path) -> Result<(), OcrError> {
    let parent = path.parent().ok_or_else(|| {
        OcrError::EngineFailure(format!(
            "Model path {} has no parent directory.",
            path.display()
        ))
    })?;
    fs::create_dir_all(parent).map_err(|error| OcrError::EngineFailure(error.to_string()))?;

    let response = ureq::get(url)
        .call()
        .map_err(|error| OcrError::EngineFailure(error.to_string()))?;
    let mut reader = response.into_reader();
    let mut temp_file = NamedTempFile::new_in(parent)
        .map_err(|error| OcrError::EngineFailure(error.to_string()))?;
    io::copy(&mut reader, &mut temp_file)
        .map_err(|error| OcrError::EngineFailure(error.to_string()))?;
    temp_file
        .persist(path)
        .map_err(|error| OcrError::EngineFailure(error.to_string()))?;
    Ok(())
}

/// Computes the lowercase hex SHA-256 checksum of a file.
fn sha256_file(path: &Path) -> Result<String, OcrError> {
    let mut file =
        fs::File::open(path).map_err(|error| OcrError::EngineFailure(error.to_string()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| OcrError::EngineFailure(error.to_string()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Verifies the SHA-256 checksum for a model file.
fn verify_sha256(path: &Path, expected: &str) -> Result<bool, OcrError> {
    let actual = sha256_file(path)?;
    Ok(actual.eq_ignore_ascii_case(expected))
}

/// Normalizes an optional hash string by trimming and lowercasing.
fn normalize_hash(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_lowercase())
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
    use image::{DynamicImage, ImageBuffer, Rgb};
    use std::fs;

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
        fs::write(&path, b"athena").unwrap();
        let hash = sha256_file(&path).unwrap();
        assert!(verify_sha256(&path, &hash).unwrap());
    }
}
