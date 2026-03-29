//! Shared model download, caching, and integrity utilities.
//!
//! These helpers are used by the OCR subsystem (and future modules) to download,
//! verify, and cache ONNX model files and related assets.

use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

/// Errors that can occur during model download or integrity verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    /// A required model file or configuration is missing.
    NotConfigured(String),
    /// A download, I/O, or hash-check failure.
    Failure(String),
}

impl fmt::Display for ModelError {
    /// Formats the error for user-facing messages.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelError::NotConfigured(msg) => write!(f, "Model not configured: {msg}"),
            ModelError::Failure(msg) => write!(f, "Model error: {msg}"),
        }
    }
}

impl std::error::Error for ModelError {}

/// Describes a single downloadable model file and its optional integrity hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelDownloadInfo {
    /// URL from which to download the model file.
    pub url: String,
    /// Optional lowercase hex SHA-256 checksum for integrity verification.
    pub sha256: Option<String>,
}

impl ModelDownloadInfo {
    /// Creates a download descriptor with the given URL and optional hash.
    pub fn new(url: impl Into<String>, sha256: Option<String>) -> Self {
        Self {
            url: url.into(),
            sha256,
        }
    }
}

/// Downloads a file from `url` to `path` using an atomic temporary-file rename.
///
/// Parent directories are created as needed. The download is written to a
/// temporary file in the same directory and then persisted via rename,
/// preventing partial files on failure.
pub fn download_to_path(url: &str, path: &Path) -> Result<(), ModelError> {
    let parent = path.parent().ok_or_else(|| {
        ModelError::Failure(format!(
            "Model path {} has no parent directory.",
            path.display()
        ))
    })?;
    fs::create_dir_all(parent).map_err(|e| ModelError::Failure(e.to_string()))?;

    let response = ureq::get(url)
        .call()
        .map_err(|e| ModelError::Failure(e.to_string()))?;
    let mut reader = response.into_reader();
    let mut temp_file =
        NamedTempFile::new_in(parent).map_err(|e| ModelError::Failure(e.to_string()))?;
    io::copy(&mut reader, &mut temp_file).map_err(|e| ModelError::Failure(e.to_string()))?;
    temp_file
        .persist(path)
        .map_err(|e| ModelError::Failure(e.to_string()))?;
    Ok(())
}

/// Computes the lowercase hex SHA-256 digest of a file.
pub fn sha256_file(path: &Path) -> Result<String, ModelError> {
    let mut file = fs::File::open(path).map_err(|e| ModelError::Failure(e.to_string()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| ModelError::Failure(e.to_string()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Returns `true` when the file at `path` matches the given hex SHA-256 hash.
pub fn verify_sha256(path: &Path, expected: &str) -> Result<bool, ModelError> {
    let actual = sha256_file(path)?;
    Ok(actual.eq_ignore_ascii_case(expected))
}

/// Normalizes an optional hash string by trimming whitespace and lowercasing.
///
/// Returns `None` for empty or whitespace-only values.
pub fn normalize_hash(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_lowercase())
}

/// Ensures a single model file exists at `path`, downloading and verifying as needed.
///
/// If the file exists and passes an integrity check (when a hash is provided), it
/// is left in place. Otherwise it is (re-)downloaded from `info.url`. Returns
/// `true` when a download was performed.
pub fn ensure_model_file(
    path: &Path,
    info: &ModelDownloadInfo,
    allow_download: bool,
) -> Result<bool, ModelError> {
    if path.exists() {
        if let Some(expected) = normalize_hash(info.sha256.as_deref()) {
            if verify_sha256(path, &expected)? {
                return Ok(false);
            }
            if !allow_download {
                return Err(ModelError::NotConfigured(format!(
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
        return Err(ModelError::NotConfigured(format!(
            "Model missing at {} and downloads are disabled.",
            path.display()
        )));
    }

    download_to_path(&info.url, path)?;
    if let Some(expected) = normalize_hash(info.sha256.as_deref())
        && !verify_sha256(path, &expected)?
    {
        let _ = fs::remove_file(path);
        return Err(ModelError::Failure(format!(
            "Downloaded model failed integrity check at {}.",
            path.display()
        )));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Verifies that `sha256_file` returns the correct digest for known content.
    #[test]
    fn sha256_file_computes_correct_digest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, b"hello world").unwrap();
        let hash = sha256_file(&path).unwrap();
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    /// Verifies that `verify_sha256` returns true for matching and false for
    /// mismatched hashes.
    #[test]
    fn verify_sha256_matches_and_rejects() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, b"hello world").unwrap();
        let correct = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        assert!(verify_sha256(&path, correct).unwrap());
        assert!(!verify_sha256(&path, "0000000000000000").unwrap());
    }

    /// Verifies that `normalize_hash` trims, lowercases, and filters empties.
    #[test]
    fn normalize_hash_handles_edge_cases() {
        assert_eq!(normalize_hash(None), None);
        assert_eq!(normalize_hash(Some("")), None);
        assert_eq!(normalize_hash(Some("   ")), None);
        assert_eq!(normalize_hash(Some(" AbCd ")), Some("abcd".to_string()));
    }

    /// Verifies that `sha256_file` returns an error for a missing file.
    #[test]
    fn sha256_file_returns_error_for_missing_file() {
        let result = sha256_file(Path::new("/nonexistent/file.bin"));
        assert!(result.is_err());
    }

    /// Verifies that `ensure_model_file` returns an error when the file is
    /// missing and downloads are disabled.
    #[test]
    fn ensure_model_file_fails_when_missing_and_no_download() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.onnx");
        let info = ModelDownloadInfo::new("http://example.com/model.onnx", None);
        let result = ensure_model_file(&path, &info, false);
        assert!(result.is_err());
    }

    /// Verifies that `ensure_model_file` succeeds without downloading when the
    /// file already exists and no hash is configured.
    #[test]
    fn ensure_model_file_skips_download_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.onnx");
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(b"dummy model bytes").unwrap();
        let info = ModelDownloadInfo::new("http://example.com/model.onnx", None);
        let downloaded = ensure_model_file(&path, &info, true).unwrap();
        assert!(!downloaded);
    }

    /// Verifies that `ModelError` Display impl formats correctly.
    #[test]
    fn model_error_display() {
        let err = ModelError::NotConfigured("missing".into());
        assert_eq!(err.to_string(), "Model not configured: missing");
        let err = ModelError::Failure("broken".into());
        assert_eq!(err.to_string(), "Model error: broken");
    }
}
