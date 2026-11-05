//! Branch-scoped chain handles plus the cached state and the
//! handle returned by [`BranchReference::acquire`].

mod reference;
mod session;
mod state;

pub use reference::{BranchReference, ProfileUpgrade, Upgraded, migrate};
pub use session::BranchSession;
pub use state::BranchState;

#[cfg(test)]
mod graph_tests;
