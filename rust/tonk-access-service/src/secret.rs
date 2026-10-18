//! The worker's secrets.
//!
//! A secret is the worker's own, set with `wrangler secret put`, or else
//! one of the account's Secrets Store, bound as `<NAME>_STORED`. Pull
//! request previews are each a worker of their own and hold no secrets
//! of their own: they share the account's, which no deployment ever
//! handles. Production, staging and dev keep their own and never read the
//! store.
//!
//! A stored secret is read asynchronously, so [`load`] reads every one
//! the worker lacks once per isolate, at each entry point, and [`secret`]
//! answers from what was loaded. A secret changed in the store is so seen
//! by isolates started after the change.

use std::cell::RefCell;
use std::collections::HashMap;

use worker::Env;

/// The secrets the worker reads.
const NAMES: [&str; 2] = ["SERVICE_SECRET_KEY", "RESEND_API_KEY"];

thread_local! {
    static STORED: RefCell<HashMap<&'static str, String>> = RefCell::new(HashMap::new());
}

/// The binding a stored secret is read through. It is not the secret's
/// own name, so a worker that holds the secret itself can bind the store
/// too without the two colliding.
fn stored_binding(name: &str) -> String {
    format!("{name}_STORED")
}

/// Read every secret the worker lacks from the Secrets Store, once per
/// isolate. A secret neither held nor stored stays missing, and
/// [`secret`] says so where it is needed.
pub async fn load(env: &Env) {
    for name in NAMES {
        if env.secret(name).is_ok() || STORED.with(|stored| stored.borrow().contains_key(name)) {
            continue;
        }
        let Ok(store) = env.secret_store(&stored_binding(name)) else {
            continue;
        };
        match store.get().await {
            Ok(Some(value)) => STORED.with(|stored| {
                stored.borrow_mut().insert(name, value);
            }),
            Ok(None) => console_warn(&format!("{name}: the Secrets Store holds no such secret")),
            Err(err) => console_warn(&format!(
                "{name}: the Secrets Store could not be read: {err}"
            )),
        }
    }
}

/// The secret named `name`: the worker's own, else the one [`load`] read
/// from the Secrets Store.
pub fn secret(env: &Env, name: &str) -> Result<String, String> {
    match env.secret(name) {
        Ok(own) => Ok(own.to_string()),
        Err(err) => STORED
            .with(|stored| stored.borrow().get(name).cloned())
            .ok_or_else(|| format!("{err}, and no {} is bound", stored_binding(name))),
    }
}

fn console_warn(message: &str) {
    worker::console_warn!("{message}");
}
