//! The session the boot hands a guest kernel: the key it acts under, and the
//! delegations to it that it claims from its host.

use std::rc::Rc;
use std::sync::{Mutex, PoisonError};

use crate::world::sapling::kernel::credential::Credential;
use crate::world::sapling::kernel::session::{Claim, Session};
use crate::world::sapling::kernel::types::Error;

struct Held {
    session: Rc<Session>,
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
        session: Rc::new(session),
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

/// The delegations to the credential that grant `claim`, which the host holds
/// or obtains. A guest kernel claims what it does not hold, and keeps what it
/// is handed.
pub async fn claim(claim: Claim) -> Result<Vec<Vec<u8>>, Error> {
    let session = with(|held| Rc::clone(&held.session));
    session.claim(claim).await
}
