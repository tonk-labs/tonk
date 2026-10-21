//! The worker's secrets.
//!
//! A secret is one of the account's Secrets Store, where the worker binds
//! it as `<NAME>_STORED`, or else the worker's own, set with `wrangler
//! secret put`. Pull request previews are each a worker of their own and
//! bind the store: they share the account's secrets, which no deployment
//! ever handles. Production, staging and dev bind no store and keep their
//! own. A bound secret wins, so a preview that once held a secret of its
//! own still reads the account's.
//!
//! A stored secret is read asynchronously, so [`load`] reads every bound
//! one once per isolate, at each entry point, and [`secret`] answers from
//! what was loaded. A secret changed in the store is so seen by isolates
//! started after the change.

use std::cell::RefCell;
use std::collections::HashMap;

use worker::Env;

/// The secrets the worker reads.
const NAMES: [&str; 2] = ["SERVICE_SECRET_KEY", "RESEND_API_KEY"];

thread_local! {
    static STORED: RefCell<HashMap<&'static str, String>> = RefCell::new(HashMap::new());
}

/// The binding a stored secret is read through. It is not the secret's
/// own name, so it never collides with a secret the worker holds itself.
fn stored_binding(name: &str) -> String {
    format!("{name}_STORED")
}

/// Read every secret the worker binds from the Secrets Store, once per
/// isolate. A bound secret the store does not hold is reported and left
/// to the worker's own.
pub async fn load(env: &Env) {
    for name in NAMES {
        if STORED.with(|stored| stored.borrow().contains_key(name)) {
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

/// The secret named `name`: the one [`load`] read from the Secrets Store,
/// else the worker's own.
pub fn secret(env: &Env, name: &str) -> Result<String, String> {
    if let Some(stored) = STORED.with(|stored| stored.borrow().get(name).cloned()) {
        return Ok(stored);
    }
    env.secret(name)
        .map(|own| own.to_string())
        .map_err(|err| format!("{err}, and no {} is bound", stored_binding(name)))
}

fn console_warn(message: &str) {
    worker::console_warn!("{message}");
}
