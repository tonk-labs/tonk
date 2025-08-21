pub mod api;
pub mod backend;
pub mod filesystem;
pub mod traversal;
pub mod types;
pub mod watcher;

// Re-export main types
pub use api::Vfs;
pub use filesystem::*;
pub use types::*;
pub use watcher::DocumentWatcher;
