use thiserror::Error;

#[derive(Error, Debug)]
pub enum RelayError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("S3 error: {0}")]
    S3(String),

    #[error("Bundle error: {0}")]
    Bundle(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Invalid manifest: {0}")]
    InvalidManifest(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("{0}")]
    Other(String),
}

impl From<anyhow::Error> for RelayError {
    fn from(err: anyhow::Error) -> Self {
        RelayError::Other(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, RelayError>;
