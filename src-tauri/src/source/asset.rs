//! Validation for the user's own `Data.rsdk`.
//!
//! The RSDKv5 container header is `b"RSDKv5"` followed by a little-endian u16
//! file count and 24-byte entries (16-byte name hash + offset + size). We check
//! that structure, not a bundled hash of proprietary game data: an expected
//! SHA-256 is accepted only if the user supplies it themselves.

use std::fmt;
use std::path::Path;

use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};

pub const EXPECTED_FILENAME: &str = "Data.rsdk";
const HEADER_LENGTH: usize = 8;
const ENTRY_LENGTH: u64 = 24;
const HASH_CHUNK: usize = 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum AssetError {
    Io(String),
    Invalid(String),
}

impl fmt::Display for AssetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(formatter, "Cannot read Data.rsdk: {message}"),
            Self::Invalid(message) => write!(formatter, "Invalid Data.rsdk: {message}"),
        }
    }
}

impl std::error::Error for AssetError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRsdkInfo {
    pub path: std::path::PathBuf,
    pub size_bytes: u64,
    pub sha256: String,
    pub file_count: u16,
}

/// Stream the file so a multi-hundred-megabyte asset is never held in memory.
pub async fn validate_data_rsdk(
    path: &Path,
    expected_sha256: Option<&str>,
) -> Result<DataRsdkInfo, AssetError> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| AssetError::Invalid("path has no usable file name".to_owned()))?;
    if !name.eq_ignore_ascii_case(EXPECTED_FILENAME) {
        return Err(AssetError::Invalid(format!(
            "expected {EXPECTED_FILENAME}, got '{name}'"
        )));
    }

    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| AssetError::Io(error.to_string()))?;
    if !metadata.is_file() {
        return Err(AssetError::Invalid("path is not a regular file".to_owned()));
    }

    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| AssetError::Io(error.to_string()))?;

    let mut header = [0u8; HEADER_LENGTH];
    file.read_exact(&mut header)
        .await
        .map_err(|error| AssetError::Io(format!("header is unreadable: {error}")))?;
    let file_count = parse_header(&header)?;

    let minimum_size = HEADER_LENGTH as u64 + ENTRY_LENGTH * u64::from(file_count);
    if metadata.len() < minimum_size {
        return Err(AssetError::Invalid(format!(
            "file is {} bytes but the header claims {file_count} entries (needs at least {minimum_size})",
            metadata.len()
        )));
    }

    file.seek(SeekFrom::Start(0))
        .await
        .map_err(|error| AssetError::Io(error.to_string()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_CHUNK];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|error| AssetError::Io(error.to_string()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = format!("{:x}", hasher.finalize());

    if let Some(expected) = expected_sha256 {
        if !digest.eq_ignore_ascii_case(expected.trim()) {
            return Err(AssetError::Invalid(format!(
                "SHA-256 mismatch: expected {}, got {digest}",
                expected.trim()
            )));
        }
    }

    Ok(DataRsdkInfo {
        path: path.to_path_buf(),
        size_bytes: metadata.len(),
        sha256: digest,
        file_count,
    })
}

fn parse_header(header: &[u8; HEADER_LENGTH]) -> Result<u16, AssetError> {
    if &header[0..4] != b"RSDK" || header[4] != b'v' || header[5] != b'5' {
        return Err(AssetError::Invalid(
            "expected an RSDKv5 signature in the first 6 bytes".to_owned(),
        ));
    }
    let file_count = u16::from_le_bytes([header[6], header[7]]);
    if file_count == 0 {
        return Err(AssetError::Invalid("header reports zero files".to_owned()));
    }
    Ok(file_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_rsdkv5_header() {
        let header = *b"RSDKv5\x02\x00";
        assert_eq!(parse_header(&header), Ok(2));
    }

    #[test]
    fn rejects_other_versions() {
        let header = *b"RSDKv4\x02\x00";
        assert!(parse_header(&header).is_err());
    }
}
