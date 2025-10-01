use thiserror::Error;

#[derive(Error, Debug)]
pub enum VfsError {
    #[error("Path not found: {0}")]
    PathNotFound(String),

    #[error("Document already exists at path: {0}")]
    DocumentExists(String),

    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("Cannot create document at root path")]
    RootPathError,

    #[error("Cannot move directory into itself or its subdirectory: {0}")]
    CircularMove(String),

    #[error("Node type mismatch: expected {expected}, got {actual}")]
    NodeTypeMismatch { expected: String, actual: String },

    #[error("Automerge error: {0}")]
    AutomergeError(#[from] automerge::AutomergeError),

    #[error("Samod error: {0}")]
    SamodError(String),

    #[error("WebSocket error: {0}")]
    WebSocketError(String),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Document not found: {0}")]
    DocumentNotFound(String),

    #[error("Invalid document structure")]
    InvalidDocumentStructure,

    #[error("Concurrent modification detected")]
    ConcurrentModification,

    #[error("Transaction failed: {0}")]
    TransactionFailed(String),
    
    #[error("Not implemented: {0}")]
    NotImplemented(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, VfsError>;

#[cfg(not(target_arch = "wasm32"))]
impl From<tokio_tungstenite::tungstenite::Error> for VfsError {
    fn from(err: tokio_tungstenite::tungstenite::Error) -> Self {
        VfsError::WebSocketError(err.to_string())
    }
}
