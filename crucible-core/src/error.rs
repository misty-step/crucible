//! Error type for loading Crucible inputs from disk.

use std::path::{Path, PathBuf};

/// Failures surfaced when reading and deserializing a Crucible input file.
///
/// Each variant carries the offending path so callers can report which input
/// failed without wrapping every call site in their own context.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The file could not be read from disk.
    #[error("failed to read {path}")]
    Read {
        /// The path that could not be read.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The file was read but did not deserialize into the expected shape.
    #[error("failed to parse {path}")]
    Parse {
        /// The path whose contents failed to deserialize.
        path: PathBuf,
        /// The underlying deserialization error.
        #[source]
        source: serde_json::Error,
    },
}

impl Error {
    pub(crate) fn read(path: &Path, source: std::io::Error) -> Self {
        Error::Read {
            path: path.to_path_buf(),
            source,
        }
    }

    pub(crate) fn parse(path: &Path, source: serde_json::Error) -> Self {
        Error::Parse {
            path: path.to_path_buf(),
            source,
        }
    }
}

/// Result alias for fallible Crucible core operations.
pub type Result<T> = std::result::Result<T, Error>;
