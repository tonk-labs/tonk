//! The session the boot hands a guest kernel: the key it acts under, the
//! authority it acts for, and the delegations between them.

use std::rc::Rc;
use std::sync::{Mutex, PoisonError};

use crate::world::sapling::kernel::credential::{Authority, Credential};
use crate::world::sapling::kernel::session::Session;

struct Held {
    session: Session,
    credential: Rc<Credential>,
}

// SAFETY: a component on WASI has no threads, so what this holds is never
// reached from another thread.
unsafe impl Send for Held {}

static SESSION: Mutex<Option<Held>> = Mutex::new(None);

/// Take up the session the boot handed over.
pub fn take_up(session: Session) {
    let credential = Rc::new(session.credential());
    *SESSION.lock().unwrap_or_else(PoisonError::into_inner) = Some(Held {
        session,
        credential,
    });
}

fn with<R>(read: impl FnOnce(&Held) -> R) -> R {
    read(
        SESSION
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .expect("the boot hands a guest kernel its session before anything reads it"),
    )
}

/// The key this worker acts under.
pub fn credential() -> Rc<Credential> {
    with(|held| Rc::clone(&held.credential))
}

/// The authority this worker acts for.
pub fn authority() -> Authority {
    with(|held| held.session.authority())
}

/// The delegations to the credential that the host hands over now: at the
/// first boot, and again as it renews them. None on a boot after the first,
/// where the kernel holds what it took up before.
pub fn proofs() -> Vec<Vec<u8>> {
    with(|held| held.session.proofs())
}

/// The delegation this worker opens a layer with: the first of its proofs, or
/// none.
pub fn chain() -> Vec<u8> {
    proofs().into_iter().next().unwrap_or_default()
}
