//! Subscriptions: a page's standing query, evaluated by the branch's
//! stack and streamed to the page as wire frames.
//!
//! The evaluation is dialog's. [`Stack::subscribe`] registers the
//! stack's subscription to a query, or attaches to the one it holds, so
//! every reader of one query on a branch reads one evaluation, keyed by
//! dialog's [`QueryHash`]. A write through the stack schedules a poll,
//! [`Stack::settle`] polls every subscription once and queues what moved
//! for each reader, and the stack holds a subscription only while a
//! reader of it does.
//!
//! What is left here is tonk's transport: a [`Connection`] is one
//! client's reader, which turns what dialog delivered into the frames a
//! page reads (a [`Frame::Snapshot`] first, then each non-empty
//! [`Frame::Delta`]); a branch keeps a [`Listener`] per connection, the
//! sender its frames go into, and after each settle sends every
//! connection its frames, outermost display first
//! ([`BranchState::deliver`](crate::BranchState::deliver)). The
//! [`Subscriber`] a caller gets back owns its connection: dropping it,
//! which is what closing the page's stream does, drops the dialog
//! reader, and the subscription goes with its last reader.
//!
//! [`Stack::subscribe`]: dialog_repository::Stack::subscribe
//! [`Stack::settle`]: dialog_repository::Stack::settle

mod connection;
mod plan;

pub(crate) use connection::Listener;
pub use connection::{Connection, Frames, Subscriber};
pub use dialog_repository::QueryHash;
pub use plan::Plan;
