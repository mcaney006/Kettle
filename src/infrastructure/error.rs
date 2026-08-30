use std::{io, path::PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum InfrastructureError {
    #[error("Homebrew was not found in a supported prefix")]
    HomebrewUnavailable,
    #[error("filesystem operation failed for {path}")]
    Filesystem {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Homebrew catalog is unavailable: {reason}")]
    CatalogUnavailable { reason: String },
    #[error("Homebrew catalog at {path} is malformed")]
    CatalogMalformed {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "both Homebrew catalog sources failed (cache: {cache}; supported fallback: {fallback})"
    )]
    CatalogFallback {
        cache: Box<InfrastructureError>,
        fallback: Box<InfrastructureError>,
    },
    #[error("could not invoke {program}")]
    Invocation {
        program: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{program} exited with status {code:?}")]
    NonZeroExit {
        program: PathBuf,
        code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    #[error("could not wait for {program}")]
    ProcessWait {
        program: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("worker pipe reader failed")]
    PipeReader(#[source] io::Error),
    #[error("network request timed out")]
    NetworkTimeout,
    #[error("network transport failed")]
    NetworkTransport(#[source] reqwest::Error),
    #[error("OAuth protocol failure: {0}")]
    OAuthProtocol(String),
    #[error("OAuth authorization was denied: {0}")]
    OAuthDenied(String),
    #[error("OAuth authorization expired")]
    OAuthExpired,
    #[error("Keychain operation failed: {0}")]
    Keychain(String),
    #[error("operation cancelled")]
    Cancelled,
    #[error("privilege helper rejected: {0}")]
    PrivilegeHelper(String),
    #[error("invalid UTF-8 from {0}")]
    InvalidUtf8(&'static str),
    #[error("JSON response from {context} was malformed")]
    Json {
        context: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

impl InfrastructureError {
    pub fn filesystem(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Filesystem {
            path: path.into(),
            source,
        }
    }
}
