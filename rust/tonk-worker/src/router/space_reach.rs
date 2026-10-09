//! Reaching a space's own worker from the person's profile.
//!
//! Where sites have origins of their own, each space's content is held by the
//! worker on the space's origin (see [`space_worker`](super::space_worker)),
//! and the worker holding the person's profile holds none of it. A command
//! the profile runs still has things to do to a space's content: this is how
//! it has that worker do them.

use tonk_schema::claim::SourceClaim;

use super::repository::CONTENT_BRANCH;
use crate::TonkWorkerError;
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
use crate::reactor::Frame;
use crate::reactor::{
    Conclusion, PeerBranchReference, PeerError, PeerFrames, PeerProvider, PeerRepositoryReference,
    Query,
};

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
        #[cfg(test)]
        if let Some(answer) = stand_in::answer(space, method, path, body) {
            return answer;
        }
        let _ = (method, path, body);
        Err(TonkWorkerError::Internal(format!(
            "could not ask {space}: this host has no worker per space"
        )))
    }
}

/// The worker on a space's own origin, as a peer the person's profile asks:
/// it holds the space, and a repository of its own. Requests go over the
/// port the profile's page opened to it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SpacePeer<'a> {
    space: &'a str,
}

/// The worker on `space`'s own origin.
pub(crate) fn peer(space: &str) -> SpacePeer<'_> {
    SpacePeer { space }
}

impl<'a> SpacePeer<'a> {
    /// The space's content branch, which that worker holds. It runs the few
    /// commands that are a space's own to run on itself.
    pub(crate) fn content(&self) -> PeerBranchReference<'a> {
        PeerRepositoryReference::new(self.space).branch(CONTENT_BRANCH)
    }

    /// That worker's own profile branch, which runs any command, naming the
    /// space. For what is a device's and not the space's: whether this
    /// device syncs it.
    pub(crate) fn profile(&self) -> PeerBranchReference<'static> {
        PeerRepositoryReference::new(PROFILE_REPOSITORY).branch(PROFILE_BRANCH)
    }

    fn path(branch: PeerBranchReference<'_>, route: &str) -> String {
        format!(
            "/api/repository/{}/branch/{}/{route}",
            branch.repository.name, branch.name
        )
    }
}

fn unreachable(error: TonkWorkerError) -> PeerError {
    PeerError::Unreachable(error.to_string())
}

fn malformed(error: impl std::fmt::Display) -> PeerError {
    PeerError::Malformed(error.to_string())
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl PeerProvider for SpacePeer<'_> {
    async fn query(
        &self,
        branch: PeerBranchReference<'_>,
        query: &Query,
    ) -> Result<Vec<Conclusion>, PeerError> {
        let body = serde_json::to_value(query).map_err(malformed)?;
        let rows = ask(
            self.space,
            "POST",
            &Self::path(branch, "query"),
            Some(&body),
        )
        .await
        .map_err(unreachable)?;
        serde_json::from_value(rows).map_err(malformed)
    }

    async fn subscribe(
        &self,
        branch: PeerBranchReference<'_>,
        query: &Query,
    ) -> Result<PeerFrames, PeerError> {
        let body = serde_json::to_value(query).map_err(malformed)?;
        subscribe(self.space, &Self::path(branch, "query"), &body).await
    }

    async fn transact(
        &self,
        branch: PeerBranchReference<'_>,
        claims: &[SourceClaim],
    ) -> Result<(), PeerError> {
        let body = serde_json::json!({ "claims": claims });
        ask(
            self.space,
            "POST",
            &Self::path(branch, "transact"),
            Some(&body),
        )
        .await
        .map(|_| ())
        .map_err(unreachable)
    }
}

/// The frames a subscription's answer carries: one per `data:` event. An
/// event that is no frame (the worker telling a page it is being replaced)
/// is passed over.
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
#[derive(Default)]
struct Events {
    unread: Vec<u8>,
}

#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
impl Events {
    /// Take in the next bytes of the answer, and give the frames they
    /// complete.
    fn read(&mut self, bytes: &[u8]) -> Vec<Result<Frame, PeerError>> {
        self.unread.extend_from_slice(bytes);
        let mut frames = Vec::new();
        while let Some(end) = self.unread.windows(2).position(|pair| pair == b"\n\n") {
            let event: Vec<u8> = self.unread.drain(..end + 2).collect();
            let Some(data) = event[..end].strip_prefix(b"data: ") else {
                continue;
            };
            match serde_json::from_slice::<serde_json::Value>(data) {
                Ok(value) if value.get("kind").is_some() => {
                    frames.push(serde_json::from_value(value).map_err(malformed));
                }
                Ok(_) => {}
                Err(error) => frames.push(Err(malformed(error))),
            }
        }
        frames
    }
}

/// Open a subscription with the worker on `space`'s own origin: `body`
/// posted to `path`, answered with an event stream that stays open. The
/// profile's worker reaches it through the `tonkSubscribeSpace` hook its
/// script defines. Dropping the frames ends the subscription there.
async fn subscribe(
    space: &str,
    path: &str,
    body: &serde_json::Value,
) -> Result<PeerFrames, PeerError> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        use futures_util::{StreamExt as _, stream};
        use js_sys::{Function, Promise, Reflect, Uint8Array};
        use wasm_bindgen::{JsCast, JsValue};
        use wasm_bindgen_futures::JsFuture;
        use wasm_streams::ReadableStream;

        let unreachable = |why: String| PeerError::Unreachable(format!("{space}: {why}"));
        let global = js_sys::global();
        let hook: Function = Reflect::get(&global, &"tonkSubscribeSpace".into())
            .ok()
            .and_then(|hook| hook.dyn_into().ok())
            .ok_or_else(|| unreachable("this worker reaches no space's worker".into()))?;
        let asked: Promise = hook
            .call3(
                &global,
                &space.into(),
                &path.into(),
                &JsValue::from_str(&body.to_string()),
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
        if !(200.0..300.0).contains(&status) {
            return Err(PeerError::Refused(format!("{space} answered {status}")));
        }
        let stream: web_sys::ReadableStream = Reflect::get(&answer, &"body".into())
            .ok()
            .and_then(|stream| stream.dyn_into().ok())
            .ok_or_else(|| malformed("the subscription has no stream"))?;
        let mut events = Events::default();
        let frames = ReadableStream::from_raw(stream)
            .into_stream()
            .flat_map(move |chunk| {
                stream::iter(match chunk.map(|bytes| bytes.dyn_into::<Uint8Array>()) {
                    Ok(Ok(bytes)) => events.read(&bytes.to_vec()),
                    Ok(Err(other)) => vec![Err(malformed(format!("{other:?}")))],
                    Err(error) => vec![Err(PeerError::Unreachable(format!("{error:?}")))],
                })
            });
        Ok(Box::pin(frames))
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        #[cfg(test)]
        if let Some(answer) = stand_in::answer(space, "SUBSCRIBE", path, Some(body)) {
            let text = answer.map_err(unreachable)?;
            let mut events = Events::default();
            let frames = events.read(text.as_str().unwrap_or_default().as_bytes());
            return Ok(Box::pin(futures_util::stream::iter(frames)));
        }
        let _ = (path, body);
        Err(PeerError::Unreachable(format!(
            "{space}: this host has no worker per space"
        )))
    }
}

/// A command as a claim: one transient concept with `fields` (each a name,
/// its attribute, and its type) applied to `parameters`.
pub(crate) fn command(
    fields: &[(&str, &str, &str)],
    parameters: serde_json::Value,
) -> Result<SourceClaim, TonkWorkerError> {
    let with: serde_json::Map<String, serde_json::Value> = fields
        .iter()
        .map(|(name, the, as_)| {
            (
                (*name).to_owned(),
                serde_json::json!({ "the": the, "as": as_ }),
            )
        })
        .collect();
    serde_json::from_value(serde_json::json!({
        "op": "assert",
        "application": {
            "predicate": { "kind": "transient", "concept": { "with": with } },
            "parameters": parameters
        }
    }))
    .map_err(|error| TonkWorkerError::Internal(format!("the command is not a claim: {error}")))
}

/// Have a space's own worker run a command: commit `claim` on `branch` of
/// what it holds (see [`SpacePeer::content`] and [`SpacePeer::profile`]).
pub(crate) async fn run(
    peer: SpacePeer<'_>,
    branch: PeerBranchReference<'_>,
    claim: SourceClaim,
) -> Result<(), TonkWorkerError> {
    branch
        .transaction()
        .apply(claim)
        .commit()
        .perform(&peer)
        .await
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))
}

/// The name a worker answers for its own profile under.
const PROFILE_REPOSITORY: &str = "profile:tonk";

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
/// can: mint an invite to this worker's space, or sign the revocation of a
/// grant on it. Answers with what the profile said back. The space's worker asks up
/// the port it was handed its delegation over, through the `tonkAskProfile`
/// hook its script defines. Fails on a host with one database, which has no
/// profile but its own.
pub(crate) async fn ask_profile(
    request: &serde_json::Value,
) -> Result<serde_json::Value, TonkWorkerError> {
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
        let answer = JsFuture::from(asked)
            .await
            .map_err(|error| unreachable(format!("{error:?}")))?;
        // What the profile answered with, when it answered with anything.
        Ok(js_sys::JSON::stringify(&answer)
            .ok()
            .and_then(|text| text.as_string())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(serde_json::Value::Null))
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        let _ = request;
        Err(TonkWorkerError::Internal(
            "could not ask the profile: this host has one database".into(),
        ))
    }
}

/// A stand-in for the workers on the spaces' own origins, for a test of
/// what a profile's worker asks of them. A host with one database has none.
#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
pub(crate) mod stand_in {
    use std::cell::RefCell;

    use crate::TonkWorkerError;

    /// What a space's worker was asked: the space, the method, the path.
    pub(crate) type Asked = (String, String, String);
    type Answer = Box<dyn Fn(&Asked) -> Result<serde_json::Value, TonkWorkerError>>;

    thread_local! {
        static WORKERS: RefCell<Option<(Answer, Vec<Asked>)>> = const { RefCell::new(None) };
    }

    /// Answer what this thread's worker asks of any space with `answer`,
    /// until [`asked`] takes the stand-in away.
    pub(crate) fn answer_with(
        answer: impl Fn(&Asked) -> Result<serde_json::Value, TonkWorkerError> + 'static,
    ) {
        WORKERS.set(Some((Box::new(answer), Vec::new())));
    }

    /// Take the stand-in away, with everything it was asked, in order.
    pub(crate) fn asked() -> Vec<Asked> {
        WORKERS.take().map(|(_, asked)| asked).unwrap_or_default()
    }

    pub(super) fn answer(
        space: &str,
        method: &str,
        path: &str,
        _body: Option<&serde_json::Value>,
    ) -> Option<Result<serde_json::Value, TonkWorkerError>> {
        WORKERS.with_borrow_mut(|workers| {
            let (answer, asked) = workers.as_mut()?;
            let ask = (space.to_owned(), method.to_owned(), path.to_owned());
            let answered = answer(&ask);
            asked.push(ask);
            Some(answered)
        })
    }
}

#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod tests {
    use futures_util::StreamExt as _;
    use serde_json::json;

    use super::{Events, Frame, command, peer, run, stand_in};
    use crate::reactor::{PeerError, Query};

    fn query() -> Query {
        serde_json::from_value(json!({
            "predicate": { "with": { "name": { "the": "xyz.tonk.probe/name", "as": "Text" } } },
            "terms": { "this": { "?": { "name": "this" } }, "name": { "?": { "name": "name" } } }
        }))
        .unwrap()
    }

    #[dialog_common::test]
    fn it_reads_a_frame_from_each_event_however_the_bytes_arrive() {
        let mut events = Events::default();
        let stream = concat!(
            r#"data: {"kind":"snapshot","conclusions":[]}"#,
            "\n\n",
            r#"data: {"control":"update-pending"}"#,
            "\n\n",
            r#"data: {"kind":"delta","asserted":[],"retracted":[]}"#,
            "\n\n",
        );
        let (first, rest) = stream.as_bytes().split_at(17);

        assert!(events.read(first).is_empty(), "an event is not half read");
        let frames = events.read(rest);

        assert!(matches!(
            frames.as_slice(),
            [Ok(Frame::Snapshot { .. }), Ok(Frame::Delta { .. })]
        ));
    }

    #[dialog_common::test]
    fn it_says_so_when_an_event_is_not_a_frame() {
        let mut events = Events::default();

        let frames = events.read(b"data: {\"kind\":\"unheard-of\"}\n\n");

        assert!(matches!(frames.as_slice(), [Err(PeerError::Malformed(_))]));
    }

    #[dialog_common::test]
    async fn it_asks_the_spaces_worker_for_what_a_peer_branch_is_asked() {
        stand_in::answer_with(|(_, method, path)| {
            Ok(match (method.as_str(), path.rsplit('/').next()) {
                ("SUBSCRIBE", _) => json!("data: {\"kind\":\"snapshot\",\"conclusions\":[]}\n\n"),
                (_, Some("query")) => json!([]),
                _ => json!({}),
            })
        });
        let space = peer("did:key:zSpace");

        let rows = space.content().query(query()).perform(&space).await;
        let mut frames = space
            .content()
            .subscribe(query())
            .perform(&space)
            .await
            .unwrap();
        let first = frames.next().await;
        let claim = command(
            &[("name", "xyz.tonk.command.rename-repository/name", "Text")],
            json!({ "name": "a name" }),
        )
        .unwrap();
        let ran = run(space, space.profile(), claim).await;

        let asked = stand_in::asked();
        assert!(rows.unwrap().is_empty());
        assert!(matches!(first, Some(Ok(Frame::Snapshot { .. }))));
        ran.unwrap();
        assert_eq!(
            asked,
            [
                (
                    "did:key:zSpace".to_owned(),
                    "POST".to_owned(),
                    "/api/repository/did:key:zSpace/branch/main/query".to_owned()
                ),
                (
                    "did:key:zSpace".to_owned(),
                    "SUBSCRIBE".to_owned(),
                    "/api/repository/did:key:zSpace/branch/main/query".to_owned()
                ),
                (
                    "did:key:zSpace".to_owned(),
                    "POST".to_owned(),
                    "/api/repository/profile:tonk/branch/main/transact".to_owned()
                ),
            ]
        );
    }

    #[dialog_common::test]
    async fn it_reaches_no_peer_on_a_host_with_one_database() {
        let space = peer("did:key:zSpace");

        let rows = space.content().query(query()).perform(&space).await;

        assert!(matches!(rows, Err(PeerError::Unreachable(_))));
    }
}
