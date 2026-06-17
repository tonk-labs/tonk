//! Subscription types — the cached state, the public subscriber
//! handle, and the reference + poll chain.

mod reference;
mod state;
mod subscriber;

pub use reference::{SubscriptionPoll, SubscriptionReference};
pub use state::QueryHash;
pub use subscriber::Subscriber;

// Crate-internal: the slot type and `Subscription` struct that
// backs the subscription map. Used by `BranchState` and the
// poll path.
pub(crate) use state::{Status, SubscriberSession, Subscription};
