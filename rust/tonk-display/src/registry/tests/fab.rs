//! The bar, as the profile library defines it.
//!
//! `<tonk-fab>` and its family are `element!:` definitions in
//! `profile.yaml`. These tests register them from the library text the
//! way a guest does, through the registry, and drive them against a
//! stand-in worker behind `fetch`: a branch's `/query` asked for an event
//! stream is a subscription the test pushes frames into, a plain one is
//! answered from a table, and claims posted to `/transact` are recorded
//! with the branch they were posted to. Tasks and navigations go through a
//! stand-in `window.tonk`.

use js_sys::{Function, Promise, Reflect};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::Element;

use super::{
    PROFILE_LIBRARY, define_from_library, defined, document, install, install_fake_host, notify,
    settle_briefly,
};

/// Every tag of the bar's family.
const FAMILY: [&str; 7] = [
    "tonk-fab",
    "tonk-share",
    "tonk-agent-panel",
    "tonk-dialog",
    "tonk-cluster",
    "tonk-field",
    "tonk-button",
];

const SPACE: &str = "did:key:zFabSpace";

/// Where a bar with no `with` of its own reads and claims on the profile.
const PROFILE: &str = "main@profile:tonk";

/// The stand-in worker and bridge, kept on `window.__fab` for the scripts
/// below.
const BRIDGE: &str = r#"
const bridge = window.tonk ?? (window.tonk = {});
const state = { subscriptions: [], claims: [], tasks: [], navigations: [], products: [],
  writes: [], answers: {}, repository: { members: [] }, refuse: false };
bridge.ready = Promise.resolve();
bridge.context = { site: 'site:fab-test', origin: 'https://tonk.test' };
bridge.task = (payload) => state.tasks.push(JSON.parse(payload));
bridge.navigate = (href) => state.navigations.push(href);
bridge.analytics = (event) => state.products.push(JSON.parse(event));
const encoder = new TextEncoder();
// Send `frame` down every live subscription whose query names `attribute`.
const send = (attribute, frame) => {
  let sent = 0;
  for (const entry of state.subscriptions) {
    if (entry.cancelled || !entry.controller || !entry.text.includes(attribute)) continue;
    entry.controller.enqueue(encoder.encode(`data: ${JSON.stringify(frame)}\n\n`));
    sent += 1;
  }
  return sent;
};
// Push all the rows that hold now, as a snapshot.
state.push = (attribute, rows) => send(attribute, { kind: 'snapshot', conclusions: rows });
// Push a change, as a delta.
state.change = (attribute, asserted, retracted) => send(attribute, { kind: 'delta', asserted, retracted });
state.live = (attribute) => state.subscriptions.filter((entry) =>
  !entry.cancelled && entry.controller && entry.text.includes(attribute));
// The claims asserting an attribute, newest last.
state.claimed = (attribute) => state.claims.filter(({ claim }) =>
  JSON.stringify(claim).includes(attribute));
// Installed afresh on every mount, over whatever another test of this
// realm left as `fetch`, so the stand-in worker answers first.
{
  const native = (window.__fabFetch ??= window.fetch.bind(window));
  window.fetch = (input, init) => {
    const state = window.__fab;
    const url = typeof input === 'string' ? input : input.url;
    const route = /^\/api\/repository\/([^/]+)\/branch\/([^/]+)\/(query|transact)$/.exec(url);
    const context = route ? { with: `${route[2]}@${route[1]}` } : null;
    if (route?.[3] === 'transact') {
      state.claims.push({ claim: JSON.parse(init.body), context });
      return state.refuse ? Promise.reject(new Error('request failed'))
        : Promise.resolve(new Response('{}', { status: 200 }));
    }
    if (route?.[3] === 'query' && String(init?.headers?.accept).includes('text/event-stream')) {
      const entry = { query: JSON.parse(init.body), context, text: init.body, controller: null, cancelled: false };
      state.subscriptions.push(entry);
      const body = new ReadableStream({
        start(controller) { entry.controller = controller; },
        cancel() { entry.cancelled = true; },
      });
      init.signal?.addEventListener('abort', () => {
        entry.cancelled = true;
        try { entry.controller.error(new DOMException('aborted', 'AbortError')); } catch {}
      });
      return Promise.resolve(new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } }));
    }
    if (route?.[3] === 'query') {
      const key = Object.keys(state.answers).find((attribute) => init.body.includes(attribute));
      return Promise.resolve(new Response(JSON.stringify(key ? state.answers[key] : []),
        { status: 200, headers: { 'content-type': 'application/json' } }));
    }
    // The repository read model a fetch of the space answers with, or a
    // worker that cannot say while it is `null`.
    if (url.startsWith('/api/repository/')) {
      return Promise.resolve(state.repository === null ? new Response('', { status: 503 })
        : new Response(JSON.stringify(state.repository),
          { status: 200, headers: { 'content-type': 'application/json' } }));
    }
    return native(input, init);
  };
}
// A clipboard that records what it was handed, and answers as told.
navigator.clipboard.write = (items) => {
  const item = items[0];
  const write = { text: null, item };
  state.writes.push(write);
  return item.getType('text/plain').then((blob) => (typeof blob === 'string' ? blob : blob.text()))
    .then((text) => { write.text = text; });
};
window.__fab = state;
return state;
"#;

/// Run `body` as a function of no arguments and return what it returns.
fn js(body: &str) -> JsValue {
    Function::new_no_args(body)
        .call0(&JsValue::NULL)
        .unwrap_or_else(|error| panic!("script failed: {error:?}\n{body}"))
}

/// Run `body` and await the promise it returns.
async fn js_await(body: &str) -> JsValue {
    let promise: Promise = js(body).dyn_into().expect("the script returns a promise");
    JsFuture::from(promise)
        .await
        .unwrap_or_else(|error| panic!("script rejected: {error:?}\n{body}"))
}

fn truthy(body: &str) -> bool {
    js(body).is_truthy()
}

/// Wait up to ten seconds for `done`, and fail naming `what` if it never
/// holds. Under a loaded machine the registry can take a while to answer.
async fn until(what: &str, done: impl Fn() -> bool) {
    for _ in 0..1000 {
        if done() {
            return;
        }
        let _ = js_await("return new Promise((resolve) => setTimeout(resolve, 10));").await;
    }
    panic!("never happened: {what}");
}

/// Put the bar's family on the branch. A tag some earlier test of this
/// realm already watched learns of its definition the way it would from
/// the host: a frame on the watch's open name subscription.
fn define_family() {
    for tag in FAMILY {
        define_from_library(PROFILE_LIBRARY, tag);
        let watch = format!("tonk-element-watch[data-tag=\"{tag}\"]");
        if document().query_selector(&watch).ok().flatten().is_some() {
            notify(tag, "name");
        }
    }
}

/// Register the bar's family from the library, install the bridge, and
/// mount `<tonk-fab>` with `attributes`, waiting until it and its
/// children have upgraded.
async fn mount(attributes: &str) -> Element {
    define_family();
    install_fake_host();
    install();
    // A test that failed left its bar behind; the scripts below find the
    // first bar in the document, which has to be this one.
    js("for (const holder of document.querySelectorAll('[data-fab-holder]')) holder.remove();");
    js(BRIDGE);
    let holder = document().create_element("div").expect("holder");
    holder
        .set_attribute("style", "position:relative;width:1200px;height:800px")
        .expect("style");
    holder.set_attribute("data-fab-holder", "").expect("mark");
    holder.set_inner_html(&format!("<tonk-fab {attributes}></tonk-fab>"));
    document()
        .body()
        .expect("body")
        .append_child(&holder)
        .expect("attach");
    let bar = holder.first_element_child().expect("the bar");
    until("the bar upgrades", || {
        defined("tonk-fab") && bar.shadow_root().is_some()
    })
    .await;
    assert!(bar.shadow_root().is_some(), "the library bar must upgrade");
    if !bar.has_attribute("latent") {
        until("its children upgrade", || {
            defined("tonk-share")
                && defined("tonk-agent-panel")
                && truthy("return typeof document.querySelector('tonk-agent-panel')?.__tonkReset === 'function'")
        }).await;
    }
    bar
}

fn unmount(bar: &Element) {
    if let Some(holder) = bar.parent_element() {
        holder.remove();
    }
}

/// `bar.shadowRoot.querySelector(selector)`, in script.
fn part(selector: &str) -> String {
    format!("document.querySelector('tonk-fab').shadowRoot.querySelector('{selector}')")
}

/// Give the account a registration, so the bar offers what an account can do.
async fn register_account() {
    until("the account subscription opens", || {
        truthy("return window.__fab.live('xyz.tonk.account/customer-email').length > 0")
    })
    .await;
    js(r#"window.__fab.push('xyz.tonk.account/customer-email',
        [{ this: 'did:key:zAccount', fields: { email: 'ada@example.com', registered_at: 1 } }]);"#);
    js(r#"window.__fab.push('xyz.tonk.account/activated-at',
        [{ this: 'did:key:zAccount', fields: { activated_at: 2 } }]);"#);
    until("the account is ready", || {
        truthy("return !document.querySelector('tonk-fab').hasAttribute('data-account-required')")
    })
    .await;
}

#[dialog_common::test]
async fn it_renders_the_v017_rail_from_the_library_with_its_children_bound_to_the_space() {
    let bar = mount(&format!("space=\"{SPACE}\"")).await;
    for selector in [
        ".header",
        "[data-cell=space]",
        ".share",
        ".publish",
        ".members",
        ".agent",
        ".home",
        "#share-panel",
        "#agent-panel",
        "#members-panel",
        ".request-layer",
    ] {
        assert!(
            truthy(&format!("return !!{}", part(selector))),
            "the rail carries {selector}"
        );
    }
    for child in ["tonk-share", "tonk-agent-panel"] {
        assert_eq!(
            bar.query_selector(child)
                .unwrap()
                .expect("the child")
                .get_attribute("space")
                .as_deref(),
            Some(SPACE),
            "<{child}> is addressed to the bar's space",
        );
    }
    // A view binds `space={…}` as a property when the element has one by
    // that name, which would leave the attribute everything reads unset.
    assert!(truthy(
        "return ['tonk-fab', 'tonk-share', 'tonk-agent-panel'].every((tag) => !('space' in document.querySelector(tag)))"
    ));
    // A bar that has not heard from the account offers to add one.
    assert!(bar.has_attribute("data-account-required"));
    assert!(truthy(&format!("return !{}.hidden", part(".login"))));
    // It is a floating, fixed-position box seated in a corner.
    assert_eq!(
        js("return document.querySelector('tonk-fab').style.position")
            .as_string()
            .as_deref(),
        Some("fixed")
    );
    assert!(
        bar.class_list().contains("fab-dock-top") && bar.class_list().contains("fab-dock-right")
    );
    unmount(&bar);
}

#[dialog_common::test]
async fn it_authors_nothing_addressed_to_a_space_on_a_latent_bar() {
    let bar = mount("latent").await;
    assert!(bar.query_selector("tonk-share").unwrap().is_none());
    assert!(bar.query_selector("tonk-agent-panel").unwrap().is_none());
    until("the site request subscription opens", || {
        truthy("return window.__fab.live('xyz.tonk.site/request').length > 0")
    })
    .await;
    assert!(
        !truthy(&format!(
            "return window.__fab.subscriptions.some((entry) => !entry.cancelled && entry.context.with !== '{PROFILE}')"
        )),
        "a latent bar opens nothing on a space's branch"
    );
    unmount(&bar);
}

#[dialog_common::test]
async fn it_paints_the_space_name_and_sync_status_from_the_space_branch() {
    let bar = mount(&format!("space=\"{SPACE}\"")).await;
    until("the name subscription opens", || {
        truthy("return window.__fab.live('xyz.tonk.repo/name').length > 0")
    })
    .await;
    assert!(truthy(&format!(
        "return window.__fab.live('xyz.tonk.repo/name')[0].context.with === 'main@{SPACE}'"
    )));
    js(&format!(
        "window.__fab.push('xyz.tonk.repo/name', [{{ this: '{SPACE}', fields: {{ name: 'Garden' }} }}]);"
    ));
    js(
        r#"window.__fab.push('xyz.tonk.sync/status', [{ this: 'state:here', fields: { status: 'sync:revoked' } }]);"#,
    );
    until("the label lands", || {
        bar.get_attribute("label").as_deref() == Some("Garden")
    })
    .await;
    until("the name paints", || {
        truthy(&format!(
            "return {}.textContent === 'Garden'",
            part(".space .n")
        ))
    })
    .await;
    assert_eq!(bar.get_attribute("state").as_deref(), Some("offline"));
    assert_eq!(
        bar.get_attribute("data-sync-status").as_deref(),
        Some("sync:revoked")
    );
    // The disc draws a hollow ring for every not-syncing state; the
    // accessible name keeps the precise one.
    register_account().await;
    until("the precise status reaches the accessible name", || {
        truthy(&format!(
            "return {}.getAttribute('aria-label').includes('revoked')",
            part(".fab")
        ))
    })
    .await;
    assert!(truthy(&format!(
        "return {}.classList.contains('offline')",
        part(".fab .disc")
    )));
    unmount(&bar);
}

#[dialog_common::test]
async fn it_folds_from_the_circle_pauses_on_alt_and_remembers_the_fold() {
    let bar = mount(&format!("space=\"{SPACE}\"")).await;
    js(&format!("{}.click();", part(".fab")));
    assert!(truthy(&format!(
        "return {}.classList.contains('collapsed')",
        part(".w")
    )));
    assert!(truthy(
        "return window.__fab.claimed('xyz.tonk.fab/collapsed').at(-1)?.claim.claims[0].application.parameters.collapsed === true"
    ));
    js(&format!(
        "{}.dispatchEvent(new MouseEvent('click', {{ bubbles: true, composed: true, altKey: true }}));",
        part(".fab")
    ));
    let pause = js("return JSON.stringify(window.__fab.claimed('xyz.tonk.pause-sync/space').at(-1)?.claim ?? null)")
        .as_string()
        .unwrap_or_default();
    assert!(
        pause.contains(SPACE),
        "alt-click pauses this space: {pause}"
    );
    assert!(
        truthy(&format!(
            "return window.__fab.claims.every((entry) => entry.context.with === '{PROFILE}')"
        )),
        "the fold and the pause are claimed on the profile's branch"
    );
    assert!(
        truthy(&format!(
            "return {}.classList.contains('collapsed')",
            part(".w")
        )),
        "a pause does not also unfold"
    );
    js(&format!("{}.click();", part(".fab")));
    assert!(!truthy(&format!(
        "return {}.classList.contains('collapsed')",
        part(".w")
    )));
    unmount(&bar);
}

#[dialog_common::test]
async fn it_opens_a_drawer_beside_the_actions_and_a_second_press_returns_to_them() {
    let bar = mount(&format!("space=\"{SPACE}\"")).await;
    js(&format!("{}.click();", part("[data-cell=space]")));
    assert!(
        !truthy(&format!("return {}.hidden", part(".run"))),
        "the space cell shows the actions"
    );
    js(&format!("{}.click();", part("[data-panel=members]")));
    assert!(!truthy(&format!(
        "return {}.hidden",
        part("#members-panel")
    )));
    assert!(truthy(&format!(
        "return {}.classList.contains('has-panel')",
        part(".w")
    )));
    assert_eq!(
        js(&format!(
            "return {}.getAttribute('aria-expanded')",
            part("[data-panel=members]")
        ))
        .as_string()
        .as_deref(),
        Some("true")
    );
    js(&format!("{}.click();", part("[data-panel=members]")));
    until("the drawer collapses", || {
        truthy(&format!("return {}.hidden", part("#members-panel")))
    })
    .await;
    assert!(
        !truthy(&format!("return {}.hidden", part(".run"))),
        "the actions stay open"
    );
    js("document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));");
    assert!(
        truthy(&format!("return {}.hidden", part(".run"))),
        "Escape closes the bar"
    );
    unmount(&bar);
}

#[dialog_common::test]
async fn it_draws_the_roster_marks_you_and_offers_publication_to_whoever_runs_the_space() {
    let bar = mount(&format!("space=\"{SPACE}\"")).await;
    register_account().await;
    until("the roster and viewer subscriptions open", || {
        truthy("return window.__fab.live('xyz.tonk.membership/member').length > 0 && window.__fab.live('xyz.tonk.session/account').length > 0")
    })
    .await;
    assert!(
        truthy(&format!(
            "return window.__fab.live('xyz.tonk.session/account')[0].context.with === 'main@{SPACE}'"
        )),
        "who is looking is read from the space's own session"
    );
    js(r#"
        window.__fab.push('xyz.tonk.membership/member', [
          { this: 'm:owner', fields: { member: 'did:key:zOwner', role: 'tonk:founder', name: 'Owner' } },
          { this: 'm:guest', fields: { member: 'did:key:zGuest', role: 'tonk:member', name: '<Guest>', invitation: 'i:1' } },
        ]);
        window.__fab.push('xyz.tonk.invitation/inviter', [{ this: 'i:1', fields: { inviter: 'did:key:zOwner' } }]);
        window.__fab.push('xyz.tonk.session/account', [{ this: 'session', fields: { account: 'did:key:zOwner' } }]);
    "#);
    until("the viewer is marked", || {
        truthy(&format!(
            "return {}?.textContent === 'Owner'",
            part(".mem-self")
        ))
    })
    .await;
    assert_eq!(
        js(&format!("return {}.textContent", part(".member-count")))
            .as_string()
            .as_deref(),
        Some("2")
    );
    assert!(
        truthy(&format!(
            "return [...{}.querySelectorAll('.mem-name')].some((name) => name.textContent === '<Guest>')",
            part(".members-list")
        )),
        "names stay plain text"
    );
    assert!(
        truthy(&format!(
            "return !!{}.querySelector('line[data-from=\"did:key:zOwner\"][data-to=\"did:key:zGuest\"]')",
            part(".members-list")
        )),
        "the guest is drawn as invited by the owner"
    );
    until("data-manages", || bar.has_attribute("data-manages")).await;
    assert!(!bar.has_attribute("data-observer"));
    // Publishing is offered to whoever runs the space, one way at a time.
    js(&format!("{}.click();", part("[data-cell=space]")));
    assert!(truthy(&format!(
        "return getComputedStyle({}).display !== 'none'",
        part(".publish")
    )));
    assert!(truthy(&format!(
        "return getComputedStyle({}).display === 'none'",
        part(".unpublish")
    )));
    js(&format!("{}.click();", part(".publish")));
    assert!(truthy(&format!(
        "return JSON.stringify(window.__fab.claimed('xyz.tonk.publish/space').at(-1)?.claim).includes('{SPACE}')"
    )));
    js(
        "window.__fab.push('xyz.tonk.invitation-execution/kind', [{ this: 'i:public', fields: { kind: 'public' } }]);",
    );
    until("data-published", || bar.has_attribute("data-published")).await;
    assert!(truthy(&format!(
        "return getComputedStyle({}).display === 'none'",
        part(".publish")
    )));
    // Signing in as another account asserts the session's new account.
    js(r#"window.__fab.change('xyz.tonk.session/account',
        [{ this: 'session', fields: { account: 'did:key:zGuest' } }],
        [{ this: 'session', fields: { account: 'did:key:zOwner' } }]);"#);
    until("the new account is marked", || {
        truthy(&format!(
            "return {}?.textContent === '<Guest>'",
            part(".mem-self")
        ))
    })
    .await;
    assert!(
        !bar.has_attribute("data-manages"),
        "a member does not run the space"
    );
    unmount(&bar);
}

#[dialog_common::test]
async fn it_reads_a_reader_of_a_published_space_as_an_observer() {
    let bar = mount(&format!("space=\"{SPACE}\"")).await;
    js("window.__fab.repository = { members: [{ did: 'did:key:zOwner', is_self: false }] };");
    until("the roster subscription opens", || {
        truthy("return window.__fab.live('xyz.tonk.membership/member').length > 0")
    })
    .await;
    js(r#"window.__fab.push('xyz.tonk.membership/member', [
        { this: 'm:owner', fields: { member: 'did:key:zOwner', role: 'tonk:founder', name: 'Owner' } }]);"#);
    until("data-observer", || bar.has_attribute("data-observer")).await;
    assert!(truthy(&format!(
        "return {}.classList.contains('observing')",
        part(".fab .disc")
    )));
    unmount(&bar);
}

#[dialog_common::test]
async fn it_reads_a_viewer_who_is_none_of_the_members_as_an_observer() {
    let bar = mount(&format!("space=\"{SPACE}\"")).await;
    // What the space's worker said at connect has landed, and from now on it
    // is not saying, so only the session can tell.
    settle_briefly().await;
    js("window.__fab.repository = null;");
    until("the roster and viewer subscriptions open", || {
        truthy("return window.__fab.live('xyz.tonk.membership/member').length > 0 && window.__fab.live('xyz.tonk.session/account').length > 0")
    })
    .await;
    js(r#"window.__fab.push('xyz.tonk.membership/member', [
        { this: 'm:owner', fields: { member: 'did:key:zOwner', role: 'tonk:founder', name: 'Owner' } }]);
        window.__fab.push('xyz.tonk.session/account', [{ this: 'session', fields: { account: 'did:key:zReader' } }]);"#);
    until("data-observer", || bar.has_attribute("data-observer")).await;
    until("the disc draws observing", || {
        truthy(&format!(
            "return {}.classList.contains('observing')",
            part(".fab .disc")
        ))
    })
    .await;
    js(
        r#"window.__fab.push('xyz.tonk.session/account', [{ this: 'session', fields: { account: 'did:key:zOwner' } }]);"#,
    );
    until("a member does not observe", || {
        !bar.has_attribute("data-observer")
    })
    .await;
    unmount(&bar);
}

#[dialog_common::test]
async fn it_copies_a_share_link_through_a_write_opened_on_the_click() {
    let bar = mount(&format!("space=\"{SPACE}\"")).await;
    register_account().await;
    js(&format!("{}.click();", part(".share")));
    assert_eq!(
        js(&format!(
            "return {}.getAttribute('data-share-state')",
            part(".share")
        ))
        .as_string()
        .as_deref(),
        Some("copying")
    );
    assert!(
        truthy("return window.__fab.writes.length === 1"),
        "the clipboard write opens inside the click"
    );
    let invite = js("return JSON.stringify(window.__fab.claimed('xyz.tonk.invite/space').at(-1)?.claim ?? null)")
        .as_string()
        .unwrap_or_default();
    assert!(
        invite.contains(SPACE) && invite.contains("xyz.tonk.command.invite/time"),
        "{invite}"
    );
    assert!(
        truthy(&format!(
            "return window.__fab.claimed('xyz.tonk.invite/space').at(-1).context.with === '{PROFILE}'"
        )),
        "the invite is claimed on the profile's branch"
    );
    // A second click while the mint is in flight mints nothing more.
    js(&format!("{}.click();", part(".share")));
    assert!(truthy(
        "return window.__fab.claimed('xyz.tonk.invite/space').length === 1"
    ));
    js(&format!(
        "window.__fab.push('xyz.tonk.invite/status', [{{ this: '{SPACE}', fields: {{ status: 'invite:granted', url: 'https://tonk.test/#invite' }} }}]);"
    ));
    until("the clipboard receives the link", || {
        truthy("return window.__fab.writes[0].text === 'https://tonk.test/#invite'")
    })
    .await;
    until("the action reads copied", || {
        truthy(&format!(
            "return {}.getAttribute('data-share-state') === 'copied'",
            part(".share")
        ))
    })
    .await;
    assert!(truthy(&format!(
        "return {}.textContent === 'copied'",
        part(".share span")
    )));
    until("the invite row is forgotten", || {
        truthy("return window.__fab.claimed('xyz.tonk.command.forget-invite/space').length === 1")
    })
    .await;
    assert!(
        truthy(
            "return window.__fab.products.some((event) => event.props.action === 'copy_share_link' && event.props.result === 'success')"
        ),
        "the attempt finishes as a success"
    );
    unmount(&bar);
}

#[dialog_common::test]
async fn it_gates_share_behind_the_trusted_account_task_and_resumes_after_it() {
    let bar = mount(&format!("space=\"{SPACE}\"")).await;
    js(&format!("{}.click();", part("[data-cell=space]")));
    js(&format!("{}.click();", part(".share")));
    assert!(
        !truthy(&format!("return {}.hidden", part("#share-panel"))),
        "an accountless share opens the gate"
    );
    assert!(truthy(
        "return window.__fab.claimed('xyz.tonk.invite/space').length === 0"
    ));
    js(&format!("{}.click();", part(".share-continue")));
    let task = js("return JSON.stringify(window.__fab.tasks.at(-1) ?? null)")
        .as_string()
        .unwrap_or_default();
    let request: serde_json::Value = serde_json::from_str(&task).expect("a task request");
    assert_eq!(request["version"], 1);
    assert_eq!(request["purpose"], "account");
    assert_eq!(request["action"], "open");
    assert_eq!(request["account"]["reason"], "needs-account");
    assert_eq!(request["account"]["space"], SPACE);
    assert!(
        request["presentation"]["anchor"]["width"]
            .as_f64()
            .unwrap_or_default()
            > 0.0
    );
    assert!(bar.has_attribute("data-task-hosted"));
    // A second ask while the ceremony is up is not a second ceremony.
    js(&format!("{}.click();", part(".login")));
    assert!(truthy("return window.__fab.tasks.length === 1"));
    js(
        "window.dispatchEvent(new CustomEvent('tonk:task-closed', { detail: { result: 'completed' } }));",
    );
    assert!(!bar.has_attribute("data-task-hosted"));
    assert!(
        !bar.has_attribute("data-account-required"),
        "a completed ceremony means an account"
    );
    assert!(
        !truthy(&format!("return {}.hidden", part(".run"))),
        "share resumes at the actions"
    );
    unmount(&bar);
}

#[dialog_common::test]
async fn it_asks_for_an_agent_invitation_when_its_drawer_opens_and_renders_the_prompt() {
    let bar = mount(&format!("space=\"{SPACE}\" label=\"Garden\"")).await;
    register_account().await;
    js(&format!("{}.click();", part("[data-cell=space]")));
    js(&format!("{}.click();", part(".agent")));
    let handoff = js("return JSON.stringify(window.__fab.claimed('xyz.tonk.agent-handoff/space').at(-1)?.claim ?? null)")
        .as_string()
        .unwrap_or_default();
    assert!(
        handoff.contains(SPACE) && handoff.contains("\"fresh\":\"new\""),
        "{handoff}"
    );
    assert_eq!(
        js(&format!("return {}.textContent", part(".agent-status")))
            .as_string()
            .as_deref(),
        Some("creating an agent invitation…")
    );
    until("the handoff subscription opens", || {
        truthy("return window.__fab.live('xyz.tonk.agent-handoff/status').length > 0")
    })
    .await;
    assert!(
        truthy(&format!(
            "return window.__fab.live('xyz.tonk.agent-handoff/status')[0].context.with === '{PROFILE}'"
        )),
        "the answer is read on the profile's branch, where it was asked"
    );
    assert!(
        truthy(&format!(
            "return window.__fab.claimed('xyz.tonk.agent-handoff/space').at(-1).context.with === '{PROFILE}'"
        )),
        "the invitation is asked for on the profile's branch"
    );
    assert!(
        truthy(&format!(
            "return window.__fab.live('xyz.tonk.agent-connection/status')[0].context.with === 'main@{SPACE}'"
        )),
        "the receipts agents leave are read on the space's branch"
    );
    js(&format!(
        "window.__fab.push('xyz.tonk.agent-handoff/status', [{{ this: '{SPACE}', fields: {{ status: 'ready', link: 'https://tonk.network/#tonk-agent-v2=secret', receipt: 'id:r1', account: 'did:key:zAccount' }} }}]);"
    ));
    until("the link is offered", || {
        truthy(&format!("return !{}.hidden", part(".agent-copy-link")))
    })
    .await;
    let prompt = js(&format!(
        "return {}.textContent",
        part("#agent-panel .panel-copytext")
    ))
    .as_string()
    .unwrap_or_default();
    assert!(prompt.contains("\"Garden\" Tonk space"), "{prompt}");
    assert!(
        prompt.contains("npx --yes @tonk/cli join 'https://tonk.network/#tonk-agent-v2=secret'"),
        "{prompt}"
    );
    // Reopening a ready drawer asks for nothing more.
    js(&format!("{}.click();", part(".agent")));
    js(&format!("{}.click();", part(".agent")));
    assert!(truthy(
        "return window.__fab.claimed('xyz.tonk.agent-handoff/space').length === 1"
    ));
    unmount(&bar);
}

#[dialog_common::test]
async fn it_offers_a_retry_when_the_agent_invitation_is_refused() {
    let bar = mount(&format!("space=\"{SPACE}\"")).await;
    register_account().await;
    js("window.__fab.refuse = true;");
    js(&format!("{}.click();", part("[data-cell=space]")));
    js(&format!("{}.click();", part(".agent")));
    assert!(truthy(
        "return window.__fab.claimed('xyz.tonk.agent-handoff/space').length === 1"
    ));
    until("the retry is offered", || {
        truthy(&format!("return !{}.hidden", part(".agent-retry")))
    })
    .await;
    assert_eq!(
        js(&format!("return {}.textContent", part(".agent-status")))
            .as_string()
            .as_deref(),
        Some("Agent invitation failed. Try again.")
    );
    unmount(&bar);
}

#[dialog_common::test]
async fn it_restores_the_remembered_corner_from_the_profile() {
    let bar = mount(&format!("space=\"{SPACE}\"")).await;
    js(r#"
        window.__fab.answers['xyz.tonk.fab/dock'] = [{ this: 'state:fab', fields: { dock: 'tonk:bottom-left' } }];
        document.querySelector('tonk-fab').restore();
    "#);
    until("the bar seats in the remembered corner", || {
        bar.class_list().contains("fab-dock-bottom") && bar.class_list().contains("fab-dock-left")
    })
    .await;
    unmount(&bar);
}

#[dialog_common::test]
async fn it_performs_a_request_left_on_its_site_but_not_one_from_before_it_connected() {
    let bar = mount(&format!("space=\"{SPACE}\"")).await;
    until("the site request subscription opens", || {
        truthy("return window.__fab.live('xyz.tonk.site/request').length > 0")
    })
    .await;
    assert!(truthy(
        "return window.__fab.live('xyz.tonk.site/request')[0].text.includes('site:fab-test')"
    ));
    js(
        r#"window.__fab.push('xyz.tonk.site/request', [{ this: 'site:fab-test', fields: { request: 'members', time: 1 } }]);"#,
    );
    settle_briefly().await;
    assert!(
        truthy(&format!("return {}.hidden", part("#members-panel"))),
        "an earlier request is not replayed"
    );
    js(
        r#"window.__fab.push('xyz.tonk.site/request', [{ this: 'site:fab-test', fields: { request: 'members', time: 2 } }]);"#,
    );
    until("the members drawer opens", || {
        truthy(&format!("return !{}.hidden", part("#members-panel")))
    })
    .await;
    assert!(!truthy(&format!(
        "return {}.hidden",
        part("#members-panel")
    )));
    unmount(&bar);
}

#[dialog_common::test]
async fn it_glides_a_dropped_bar_to_the_nearest_edge_and_remembers_the_corner() {
    let bar = mount(&format!("space=\"{SPACE}\"")).await;
    let snap = js(r#"
        const bar = document.querySelector('tonk-fab');
        const header = bar.shadowRoot.querySelector('.header');
        const rect = header.getBoundingClientRect();
        const at = (x, y, type, buttons) => new PointerEvent(type, { bubbles: true, composed: true,
          clientX: x, clientY: y, button: 0, buttons, pointerId: 7, pointerType: 'mouse' });
        let snap = null;
        bar.addEventListener('fabb-snap', (event) => { snap = event.detail; }, { once: true });
        const x = rect.left + 60, y = rect.top + 20;
        header.dispatchEvent(at(x, y, 'pointerdown', 1));
        window.dispatchEvent(at(x - 200, y + 300, 'pointermove', 1));
        window.dispatchEvent(at(30, y + 300, 'pointermove', 1));
        window.dispatchEvent(at(30, y + 300, 'pointerup', 0));
        return snap;
    "#);
    assert!(!snap.is_null(), "a drop announces where it settled");
    assert_eq!(
        Reflect::get(&snap, &"edge".into())
            .unwrap()
            .as_string()
            .as_deref(),
        Some("left")
    );
    assert!(!bar.has_attribute("dragging"));
    assert!(
        !bar.has_attribute("flip"),
        "a bar on the left edge is not mirrored"
    );
    let dock = js("return window.__fab.claimed('xyz.tonk.fab/dock').at(-1)?.claim.claims[0].application.parameters.dock")
        .as_string();
    assert!(
        dock.is_some_and(|dock| dock.ends_with("-left")),
        "the nearest corner is remembered"
    );
    // The release's trailing click is not a fold.
    js(&format!("{}.click();", part(".fab")));
    assert!(!truthy(&format!(
        "return {}.classList.contains('collapsed')",
        part(".w")
    )));
    unmount(&bar);
}

#[dialog_common::test]
async fn it_lends_its_seat_to_a_task_and_gives_it_back_as_it_was() {
    let bar = mount(&format!("space=\"{SPACE}\"")).await;
    js(&format!("{}.click();", part("[data-cell=space]")));
    js(
        "window.__content = document.createElement('section'); window.__content.innerHTML = '<p>hello</p><button data-fabb-result=\"keep\">keep</button>'; document.body.append(window.__content);",
    );
    let outcome = js(
        "window.__outcome = document.querySelector('tonk-fab').present(window.__content, { heading: 'a question', dismissible: true }); return true;",
    );
    assert!(outcome.is_truthy());
    assert!(truthy(
        "return document.querySelector('tonk-fab').requesting === true"
    ));
    assert!(truthy(
        "return window.__content.parentElement === document.querySelector('tonk-fab') && window.__content.slot === 'request'"
    ));
    assert_eq!(
        js(&format!("return {}.textContent", part(".task-title")))
            .as_string()
            .as_deref(),
        Some("a question")
    );
    // Only one at a time.
    let busy = js_await(r#"return document.querySelector('tonk-fab').present(document.createElement('p')).then(() => 'presented', (error) => error.message);"#).await;
    assert!(
        busy.as_string()
            .unwrap_or_default()
            .contains("Resolve the current FABB request")
    );
    js("window.__content.querySelector('[data-fabb-result]').click();");
    let result = js_await("return window.__outcome;").await;
    assert_eq!(result.as_string().as_deref(), Some("keep"));
    assert!(truthy(
        "return window.__content.parentElement === document.body && !window.__content.hasAttribute('slot')"
    ));
    assert!(truthy(
        "return document.querySelector('tonk-fab').requesting === false"
    ));
    assert!(
        !truthy(&format!("return {}.hidden", part(".run"))),
        "the open actions come back"
    );
    js("window.__content.remove();");
    unmount(&bar);
}

/// The invariant the worker enforces: a parameter the inlined concept does
/// not declare fails the whole transact, and a declared field with no value
/// makes the assert match nothing. The element builds both halves from
/// one table, so neither can drift from the other.
#[dialog_common::test]
async fn it_declares_exactly_the_fields_each_command_sets() {
    let bar = mount(&format!("space=\"{SPACE}\"")).await;
    // A value the command does not declare builds no claim at all, so a
    // transact of it sends nothing.
    assert!(truthy(
        r#"
        const bar = document.querySelector('tonk-fab');
        const before = window.__fab.claims.length;
        const claim = bar.command('pause-sync', { time: 1, space: 'did:key:z', marker: 'x' });
        bar.transact(claim);
        return claim === undefined && window.__fab.claims.length === before;
    "#
    ));
    assert!(truthy(
        r#"
        const share = document.querySelector('tonk-share');
        for (const [element, name, values] of [
          [document.querySelector('tonk-fab'), 'pause-sync', { time: 1, space: 'did:key:z' }],
          [document.querySelector('tonk-fab'), 'dock', { this: 'state:fab', dock: 'tonk:top-left' }],
          [share, 'invite', { time: 1, space: 'did:key:z' }],
          [share, 'enable-sync', { time: 1, space: 'did:key:z' }],
          [document.querySelector('tonk-agent-panel'), 'handoff', { time: 1, space: 'did:key:z', fresh: undefined }],
        ]) {
          const { predicate, parameters } = element.command(name, values).claims[0].application;
          const declared = Object.keys(predicate.concept.with).sort().join();
          const set = Object.keys(parameters).filter((field) => field !== 'this').sort().join();
          if (!declared || declared !== set) return false;
        }
        return true;
    "#
    ));
    unmount(&bar);
}

#[dialog_common::test]
async fn it_cycles_tab_across_shadow_and_slotted_dialog_controls() {
    define_family();
    install_fake_host();
    install();
    let host = document()
        .create_element("tonk-dialog")
        .expect("dialog host");
    host.set_inner_html(
        r#"<button type="button">body control</button>
           <button slot="actions" type="button">cancel</button>
           <button slot="actions" type="button">remove space</button>"#,
    );
    document()
        .body()
        .expect("body")
        .append_child(&host)
        .expect("append");
    until("the dialog upgrades", || {
        defined("tonk-dialog") && host.shadow_root().is_some()
    })
    .await;
    js("document.querySelector('tonk-dialog').show();");
    assert!(truthy(
        "return document.querySelector('tonk-dialog').open === true"
    ));
    let wrapped = js(r#"
        const host = document.querySelector('tonk-dialog');
        const close = host.shadowRoot.querySelector('.x');
        const last = host.querySelectorAll('[slot=actions]')[1];
        const tab = (target, shiftKey) => target.dispatchEvent(new KeyboardEvent('keydown',
          { key: 'Tab', shiftKey, bubbles: true, composed: true, cancelable: true }));
        last.focus();
        tab(last, false);
        const forward = host.shadowRoot.activeElement === close;
        tab(close, true);
        const back = document.activeElement === last;
        host.close();
        return forward && back && host.open === false;
    "#);
    assert!(
        wrapped.is_truthy(),
        "Tab wraps from the last action to the close, and back"
    );
    host.remove();
}
