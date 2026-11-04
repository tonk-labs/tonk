//! The session the boot hands a guest kernel: the key it acts under, the
//! representative it acts through, and the persona it acts for.

use std::rc::Rc;
use std::sync::{Mutex, PoisonError};

use crate::world::sapling::kernel::credential::{
    Access, Credential, CredentialBackup, Persona, Representative,
};
use crate::world::sapling::kernel::session::{PersonaSession, Session};
use crate::world::sapling::kernel::types::Error;

/// The persona a worker acts for: the persona's own handle, which only the
/// persona's own worker is given, or its DID.
enum Acting {
    Persona(Rc<PersonaSession>),
    Space(String),
}

struct Held {
    credential: Rc<Credential>,
    representative: Rc<Representative>,
    acting: Acting,
}

// SAFETY: a component on WASI has no threads, so what this holds is never
// reached from another thread.
unsafe impl Send for Held {}

static SESSION: Mutex<Option<Held>> = Mutex::new(None);

/// Take up the session the boot handed over.
pub fn take_up(session: Session) {
    let held = match session {
        Session::Persona(session) => Held {
            credential: Rc::new(session.credential()),
            representative: Rc::new(session.representative()),
            acting: Acting::Persona(Rc::new(session)),
        },
        Session::Space(session) => Held {
            credential: Rc::new(session.credential()),
            representative: Rc::new(session.representative()),
            acting: Acting::Space(session.persona()),
        },
    };
    *SESSION.lock().unwrap_or_else(PoisonError::into_inner) = Some(held);
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

/// What this worker acts through, such as a device.
pub fn representative() -> Rc<Representative> {
    with(|held| Rc::clone(&held.representative))
}

/// The persona, as it stands now, if this is the persona's own worker.
pub fn persona() -> Option<Persona> {
    with(|held| match &held.acting {
        Acting::Persona(session) => Some(session.persona()),
        Acting::Space(_) => None,
    })
}

/// The DID of the persona this worker acts for.
pub fn persona_did() -> String {
    with(|held| match &held.acting {
        Acting::Persona(session) => match session.persona() {
            Persona::Provisional(persona) => persona.did(),
            Persona::Permanent(persona) => persona.did(),
        },
        Acting::Space(did) => did.clone(),
    })
}

/// A delegation of `access` to this worker's credential, from the persona
/// through the representative. A guest kernel asks for what it does not
/// hold, and keeps what it is handed.
pub async fn delegate(access: Access) -> Result<Vec<u8>, Error> {
    let (representative, audience) =
        with(|held| (Rc::clone(&held.representative), held.credential.did()));
    representative.delegate(audience, access).await
}

/// `secret` as a backup restored by this worker's credential, which it kept
/// when it backed a key up to that credential.
pub fn backup_to_credential(secret: Vec<u8>) -> CredentialBackup {
    CredentialBackup {
        custodian: credential().did(),
        secret,
    }
}
