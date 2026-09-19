//! Automerge documents in tonk.
//!
//! An automerge document is an entity plus one memory cell. The cell is
//! the object store: it holds the serialized changes of every branch.
//! Each branch holds the reference: a small `document/heads` claim with
//! the automerge heads that branch is at. No document bytes enter the
//! search tree.
//!
//! Everything here is host-neutral. The service worker and the native
//! CLI call the same functions and differ only in when they call them.

pub mod cell;
pub mod engine;
pub mod formula;
pub mod session;
pub mod sync;

pub use cell::{CellError, LocalCell, RemoteCell, Transport};
pub use engine::{
    ChangeInfo, Content, DiffOp, Document, DocumentError, Edit, Format, Sheet, Stamp, Table,
};
pub use session::{DocumentEnv, SessionError, Snapshot, Written};
pub use sync::{Marker, Outcome, sync_pass};
