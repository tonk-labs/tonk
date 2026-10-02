//! Reaching a space's own worker from the person's profile.
//!
//! Where sites have origins of their own, each space's content is held by the
//! worker on the space's origin (see [`space_worker`](super::space_worker)),
//! and the worker holding the person's profile holds none of it. A command
//! the profile runs still has things to do to a space's content: this is how
//! it has that worker do them.

use super::repository::CONTENT_BRANCH;
use crate::TonkWorkerError;

/// Ask the worker on `space`'s own origin, which holds the space's content.
///
/// Where sites have origins of their own, the person's profile holds none of
/// a space's content, and what one of its commands does to that content is
/// done by the space's worker. The profile's worker reaches it over a port
/// its page opens, through the `tonkAskSpace` hook its script defines.
/// Answers with the response's body, and fails for a response that is not a
/// success, and where there is no such hook: on a host with one database.
pub(crate) async fn ask(
    space: &str,
    method: &str,
    path: &str,
    body: Option<&serde_json::Value>,
) -> Result<serde_json::Value, TonkWorkerError> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        use js_sys::{Function, Promise, Reflect};
        use wasm_bindgen::{JsCast, JsValue};
        use wasm_bindgen_futures::JsFuture;

        let unreachable =
            |why: String| TonkWorkerError::Internal(format!("could not ask {space}: {why}"));
        let global = js_sys::global();
        let hook: Function = Reflect::get(&global, &"tonkAskSpace".into())
            .ok()
            .and_then(|hook| hook.dyn_into().ok())
            .ok_or_else(|| unreachable("this worker reaches no space's worker".into()))?;
        let body = body.map_or(JsValue::NULL, |body| JsValue::from_str(&body.to_string()));
        let asked: Promise = hook
            .apply(
                &global,
                &js_sys::Array::of4(&space.into(), &method.into(), &path.into(), &body),
            )
            .ok()
            .and_then(|asked| asked.dyn_into().ok())
            .ok_or_else(|| unreachable("the hook did not answer with a promise".into()))?;
        let answer = JsFuture::from(asked)
            .await
            .map_err(|error| unreachable(format!("{error:?}")))?;
        let status = Reflect::get(&answer, &"status".into())
            .ok()
            .and_then(|status| status.as_f64())
            .unwrap_or_default();
        let text = Reflect::get(&answer, &"body".into())
            .ok()
            .and_then(|text| text.as_string())
            .unwrap_or_default();
        if !(200.0..300.0).contains(&status) {
            return Err(unreachable(format!("it answered {status}: {text}")));
        }
        Ok(serde_json::from_str(&text).unwrap_or(serde_json::Value::Null))
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        let _ = (method, path, body);
        Err(TonkWorkerError::Internal(format!(
            "could not ask {space}: this host has no worker per space"
        )))
    }
}

/// Which of a space's own worker's branches a command is committed on.
///
/// That worker is this same crate under a profile of its own, so it has both
/// vocabularies a worker has (see
/// [`CommandProviders`](super::command::CommandProviders)).
#[derive(Clone, Copy, Debug)]
pub(crate) enum Surface {
    /// The space's content branch, which runs the few commands that are a
    /// space's own to run on itself.
    Space,
    /// That worker's own profile branch, which runs any command, naming the
    /// space. For what is a device's and not the space's: whether this
    /// device syncs it.
    Profile,
}

/// A command as a transact request: one transient concept with `fields`
/// (each a name, its attribute, and its type) applied to `parameters`.
pub(crate) fn command(
    fields: &[(&str, &str, &str)],
    parameters: serde_json::Value,
) -> serde_json::Value {
    let with: serde_json::Map<String, serde_json::Value> = fields
        .iter()
        .map(|(name, the, as_)| {
            (
                (*name).to_owned(),
                serde_json::json!({ "the": the, "as": as_ }),
            )
        })
        .collect();
    serde_json::json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": { "kind": "transient", "concept": { "with": with } },
                "parameters": parameters
            }
        }]
    })
}

/// Have `space`'s own worker run a command: commit `claims` (a transact
/// request, see [`command`]) on the branch `surface` names.
pub(crate) async fn run(
    space: &str,
    surface: Surface,
    claims: &serde_json::Value,
) -> Result<(), TonkWorkerError> {
    let path = match surface {
        Surface::Space => format!("/api/repository/{space}/branch/{CONTENT_BRANCH}/transact"),
        Surface::Profile => format!("/api/profile/branch/{PROFILE_BRANCH}/transact"),
    };
    ask(space, "POST", &path, Some(claims)).await.map(|_| ())
}

/// The branch a space's own worker keeps its profile on. It has one profile,
/// made on its origin's first boot, and never another branch of it.
const PROFILE_BRANCH: &str = "main";

/// Tell the worker of `space`, or of every space with `None`, that what its
/// profile told it has changed: where the space syncs, or which account the
/// profile acts for. It takes up a new delegation and the new terms with it.
/// A worker that is not running learns of it when it next starts. Nothing to
/// tell on a host with one database.
pub(crate) fn changed(space: Option<&str>) {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        use js_sys::{Function, Reflect};
        use wasm_bindgen::{JsCast, JsValue};

        let global = js_sys::global();
        if let Some(hook) = Reflect::get(&global, &"tonkSpaceChanged".into())
            .ok()
            .and_then(|hook| hook.dyn_into::<Function>().ok())
        {
            let space = space.map_or(JsValue::NULL, JsValue::from_str);
            let _ = hook.call1(&global, &space);
        }
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        let _ = space;
    }
}

/// Have `space`'s own worker forget the space: remove everything its origin
/// stored, and itself. For a space the person removed from this device,
/// whose content this worker never held. Best effort, like the removal of
/// local storage it stands in for: a worker that cannot be reached leaves
/// bytes nothing shows, on an origin nothing opens.
pub(crate) async fn forget(space: &str) {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        use js_sys::{Function, Promise, Reflect};
        use wasm_bindgen::{JsCast, JsValue};
        use wasm_bindgen_futures::JsFuture;

        let global = js_sys::global();
        let Some(hook) = Reflect::get(&global, &"tonkForgetSpace".into())
            .ok()
            .and_then(|hook| hook.dyn_into::<Function>().ok())
        else {
            return;
        };
        let forgotten = hook
            .call1(&global, &JsValue::from_str(space))
            .ok()
            .and_then(|forgotten| forgotten.dyn_into::<Promise>().ok());
        if let Some(forgotten) = forgotten
            && let Err(error) = JsFuture::from(forgotten).await
        {
            tonk_common::log!("{space} was not forgotten by its own worker: {error:?}");
        }
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        let _ = space;
    }
}

/// Ask the person's profile, from a space's own worker, to do what only it
/// can: mint an invite to this worker's space. The space's worker asks up
/// the port it was handed its delegation over, through the `tonkAskProfile`
/// hook its script defines. Fails on a host with one database, which has no
/// profile but its own.
pub(crate) async fn ask_profile(request: &serde_json::Value) -> Result<(), TonkWorkerError> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        use js_sys::{Function, Promise, Reflect};
        use wasm_bindgen::JsCast;
        use wasm_bindgen_futures::JsFuture;

        let unreachable =
            |why: String| TonkWorkerError::Internal(format!("could not ask the profile: {why}"));
        let global = js_sys::global();
        let hook: Function = Reflect::get(&global, &"tonkAskProfile".into())
            .ok()
            .and_then(|hook| hook.dyn_into().ok())
            .ok_or_else(|| unreachable("this worker answers to no profile".into()))?;
        let request = js_sys::JSON::parse(&request.to_string())
            .map_err(|error| unreachable(format!("{error:?}")))?;
        let asked: Promise = hook
            .call1(&global, &request)
            .ok()
            .and_then(|asked| asked.dyn_into().ok())
            .ok_or_else(|| unreachable("the hook did not answer with a promise".into()))?;
        JsFuture::from(asked)
            .await
            .map(|_| ())
            .map_err(|error| unreachable(format!("{error:?}")))
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        let _ = request;
        Err(TonkWorkerError::Internal(
            "could not ask the profile: this host has one database".into(),
        ))
    }
}
