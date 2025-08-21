pub mod error;
pub mod sync;
pub mod vfs;

// Re-export main types
pub use error::{Result, VfsError};
pub use sync::SyncEngine;
pub use vfs::{
    DirNode, DocNode, DocumentWatcher, NodeType, RefNode, Timestamps, Vfs, VfsEvent,
    VirtualFileSystem,
};
