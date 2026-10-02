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

/// Commit `claims` (a transact request) on `space`'s content, in the space's
/// own worker. A command among them runs there.
pub(crate) async fn transact(
    space: &str,
    claims: &serde_json::Value,
) -> Result<(), TonkWorkerError> {
    let path = format!("/api/repository/{space}/branch/{CONTENT_BRANCH}/transact");
    ask(space, "POST", &path, Some(claims)).await.map(|_| ())
}

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
