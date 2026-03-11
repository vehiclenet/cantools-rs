//! Portable capture readers and writers.

mod asc;
mod blf;
mod mf4;

use std::path::Path;

use cantools_core::CaptureEvent;
use thiserror::Error;

pub use asc::{read_asc, write_asc};
pub use blf::{read_blf, write_blf};
pub use mf4::{read_mf4, write_mf4};

/// Supported persisted capture formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// ASAM MDF 4 capture file.
    Mf4,
    /// Vector ASC text log.
    Asc,
    /// Vector BLF binary log.
    Blf,
}

impl Format {
    /// Infer the format from a path extension.
    pub fn from_path(path: &Path) -> Result<Self> {
        match path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
        {
            Some(ext) if ext == "mf4" => Ok(Self::Mf4),
            Some(ext) if ext == "asc" => Ok(Self::Asc),
            Some(ext) if ext == "blf" => Ok(Self::Blf),
            _ => Err(CodecError::UnknownFormat(path.display().to_string())),
        }
    }
}

/// Explicit note about fidelity loss or adaptation while translating formats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FidelityNote {
    /// Event index when the note applies to a specific event.
    pub event_index: Option<usize>,
    /// Short field or area label.
    pub field: &'static str,
    /// Human-readable detail.
    pub detail: String,
}

/// Result of reading a capture file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadReport {
    /// Parsed events.
    pub events: Vec<CaptureEvent>,
    /// Fidelity notes emitted while decoding.
    pub notes: Vec<FidelityNote>,
}

/// Result of writing a capture file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WriteReport {
    /// Fidelity notes emitted while encoding.
    pub notes: Vec<FidelityNote>,
}

/// Codec errors.
#[derive(Debug, Error)]
pub enum CodecError {
    /// I/O failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// A format-specific parse error occurred.
    #[error("parse error: {0}")]
    Parse(String),
    /// The path extension does not map to a supported format.
    #[error("unknown capture format for path {0}")]
    UnknownFormat(String),
    /// Core value validation failed while reconstructing an event.
    #[error(transparent)]
    Core(#[from] cantools_core::CoreError),
}

/// Result type for codec operations.
pub type Result<T> = std::result::Result<T, CodecError>;

/// Read capture events from a file and infer the format from the extension.
pub fn read_path(path: impl AsRef<Path>) -> Result<ReadReport> {
    let path = path.as_ref();
    match Format::from_path(path)? {
        Format::Asc => read_asc(path),
        Format::Blf => read_blf(path),
        Format::Mf4 => read_mf4(path),
    }
}

/// Write capture events to a file and infer the format from the extension.
pub fn write_path(path: impl AsRef<Path>, events: &[CaptureEvent]) -> Result<WriteReport> {
    let path = path.as_ref();
    match Format::from_path(path)? {
        Format::Asc => write_asc(path, events),
        Format::Blf => write_blf(path, events),
        Format::Mf4 => write_mf4(path, events),
    }
}
