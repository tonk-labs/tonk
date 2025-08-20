pub mod documents;
pub mod engine;
pub mod error;
pub mod sync;
pub mod utils;
pub mod vfs;

// Re-export main types
pub use engine::{SyncEngine, SyncEngineConfig};
pub use error::{Result, VfsError};
pub use sync::SyncEngine as VfsSyncEngine;
pub use vfs::{DirNode, DocNode, NodeType, RefNode, Timestamps, Vfs, VfsEvent, VirtualFileSystem};
