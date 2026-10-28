//! Typed error surface for `taste-core`.
//!
//! Binaries are free to wrap these in `anyhow`; the library itself never does,
//! so callers can match on failure modes such as [`Error::MalformedMarkers`].

use std::path::PathBuf;

/// Result alias used throughout `taste-core`.
pub type Result<T> = std::result::Result<T, Error>;

/// Every way a `taste-core` operation can fail.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("i/o error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("i/o error: {0}")]
    BareIo(#[from] std::io::Error),

    #[error("malformed json in {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("json error: {0}")]
    BareJson(#[from] serde_json::Error),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("invalid glob pattern {pattern:?}: {source}")]
    Glob {
        pattern: String,
        #[source]
        source: globset::Error,
    },

    #[error("could not acquire the OpenTaste lock at {path}: {source}")]
    Lock {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("no OpenTaste project found at or above {start}; run `taste init` first")]
    ProjectRootNotFound { start: PathBuf },

    #[error("could not determine the current user's home directory")]
    HomeDirNotFound,

    #[error(
        "{path} has an unbalanced OpenTaste marker block; \
         fix or remove the markers by hand so no content is lost"
    )]
    MalformedMarkers { path: PathBuf },

    #[error("{value:?} is not a valid {kind}")]
    InvalidEnumValue { kind: &'static str, value: String },
}

impl Error {
    /// Attach a path to an [`std::io::Error`] for a readable message.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    /// Attach a path to a [`serde_json::Error`] for a readable message.
    pub fn json(path: impl Into<PathBuf>, source: serde_json::Error) -> Self {
        Error::Json {
            path: path.into(),
            source,
        }
    }

    /// Attach the offending pattern to a [`globset::Error`].
    pub fn glob(pattern: impl Into<String>, source: globset::Error) -> Self {
        Error::Glob {
            pattern: pattern.into(),
            source,
        }
    }
}
