//! Branch-scoped chain handles plus the cached state and the
//! handle returned by [`BranchReference::acquire`].

mod reference;
mod session;
mod state;

pub use reference::BranchReference;
pub use session::BranchSession;
pub use state::BranchState;
