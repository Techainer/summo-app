//! One error type for the whole workspace.
//!
//! Crates add variants here rather than defining their own error enums, so a failure can travel
//! from the ONNX runtime all the way to a JSON payload without a chain of `From` conversions.

use std::path::PathBuf;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("io error: {0}")]
    BareIo(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// A model manifest was syntactically valid but semantically unusable.
    #[error("invalid manifest {id}: {reason}")]
    InvalidManifest { id: String, reason: String },

    #[error("model not found: {0}")]
    ModelNotFound(String),

    /// Downloaded bytes did not match the manifest digest — never fall back to using them.
    #[error("checksum mismatch for {file}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        file: String,
        expected: String,
        actual: String,
    },

    #[error("registry error: {0}")]
    Registry(String),

    #[error("download failed for {url}: {reason}")]
    Download { url: String, reason: String },

    #[error("audio device error: {0}")]
    Audio(String),

    #[error("no audio input device available")]
    NoInputDevice,

    /// The OS denied microphone / screen-capture permission. Callers surface a per-OS remedy.
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("vad error: {0}")]
    Vad(String),

    #[error("asr error: {0}")]
    Asr(String),

    #[error("unsupported runtime: {0}")]
    UnsupportedRuntime(String),

    #[error("store error: {0}")]
    Store(String),

    #[error("vault error: {0}")]
    Vault(String),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Attach a path to an [`std::io::Error`]; bare io errors are near-useless in logs.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }

    /// Whether retrying the same operation could plausibly succeed.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Download { .. } | Self::Registry(_) => true,
            Self::Io { source, .. } | Self::BareIo(source) => matches!(
                source.kind(),
                std::io::ErrorKind::Interrupted
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::WouldBlock
                    | std::io::ErrorKind::ConnectionReset
            ),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_mismatch_is_not_transient() {
        let err = Error::ChecksumMismatch {
            file: "encoder.onnx".into(),
            expected: "aaa".into(),
            actual: "bbb".into(),
        };
        assert!(
            !err.is_transient(),
            "a corrupt download must never be retried into use"
        );
    }

    #[test]
    fn network_failures_are_transient() {
        let err = Error::Download {
            url: "https://cdn.summo.app/x".into(),
            reason: "reset".into(),
        };
        assert!(err.is_transient());
    }

    #[test]
    fn io_error_keeps_path_in_message() {
        let err = Error::io("/tmp/model.onnx", std::io::Error::other("boom"));
        assert!(err.to_string().contains("/tmp/model.onnx"));
    }
}
