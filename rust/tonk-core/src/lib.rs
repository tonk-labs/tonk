pub mod bundle;
pub mod error;
pub mod tonk_core;
pub mod vfs;
pub mod websocket;

pub use bundle::{Bundle, BundlePath};
#[cfg(target_arch = "wasm32")]
pub use tonk_core::ConnectionState;
pub use tonk_core::{StorageConfig, TonkCore, TonkCoreBuilder};
#[cfg(not(target_arch = "wasm32"))]
pub use websocket::ConnectionHandle;
pub use vfs::{
    DirNode, DocNode, DocumentWatcher, NodeType, RefNode, Timestamps, VfsEvent, VirtualFileSystem,
};

#[cfg(target_arch = "wasm32")]
pub mod wasm;
