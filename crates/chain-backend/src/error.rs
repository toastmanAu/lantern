//! Error type for the chain backend.
//!
//! RPC text passes through because it originates at a node, never from key
//! material. `Spawn` carries a path or an OS message and nothing else.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("transport error: {0}")]
    Transport(String),

    #[error("node returned error {code}: {message}")]
    Rpc { code: i64, message: String },

    #[error("the active backend does not support {0}")]
    Unsupported(&'static str),

    #[error("backend is not ready")]
    NotReady,

    #[error("could not start the light client: {0}")]
    Spawn(String),

    #[error("timed out waiting for the backend")]
    Timeout,

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("backend configuration file is corrupt or has an unsupported version")]
    Corrupt,

    #[error("no backend profile with that id")]
    ProfileNotFound,
}

#[cfg(test)]
mod tests {
    use super::BackendError;

    #[test]
    fn rpc_errors_report_code_and_message() {
        let e = BackendError::Rpc {
            code: -32601,
            message: "Method not found".into(),
        };
        assert_eq!(
            e.to_string(),
            "node returned error -32601: Method not found"
        );
    }

    #[test]
    fn io_errors_convert() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "x");
        assert!(matches!(BackendError::from(io), BackendError::Io(_)));
    }
}
