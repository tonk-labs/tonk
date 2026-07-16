# FAB Web Component Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `<tonk-fab>` a self-contained web component that owns its markup, stylesheet, reads and writes — so it renders correctly against a space branch seeded by any past `core.yaml`.

**Architecture:** The FAB stops mounting `core.yaml`-seeded views. Reads become inline-predicate subscriptions over raw attribute URIs, issued by `ui-` child elements that each carry their own `with="main@{did}"` (the `<ui-sync-status>` pattern). Writes become inlined command descriptors dispatched routeless via `window.tonk.transact`, naming the target space as a parameter (the `pause_claim_json` / `PauseSyncHandler` pattern from #572). This works because handlers match commands by attribute URI, not by seeded-descriptor identity.

**Tech Stack:** Rust + wasm-bindgen, `custom_elements::CustomElement`, `tonk_host::consumer` for subscriptions, `web-sys`. Standard library YAML in `rust/tonk-core/assets/library/`.

Spec: `docs/superpowers/specs/2026-07-16-fab-web-component-design.md`

## Global Constraints

- **No `mod.rs`.** Use `foo.rs` + `foo/` form everywhere.
- **Test naming:** `it_does_x`, grouped by behaviour. Use `#[dialog_common::test]` in crates that have it; `tonk-fab/src/logic.rs` uses plain `#[test]` — follow the file. `tonk-worker` uses plain `autotests` (no `[[test]]` entries) — a new `tests/*.rs` file is picked up automatically.
- **No emojis** in code, commits, or output.
- **Commits:** Conventional Commits — `type(scope): subject`, imperative, lowercase, no trailing period, under ~72 chars.
- **PRs target `origin/staging`**, not `main`.
- **Lint gate:** `cargo clippy --workspace --all-targets --all-features` + `cargo fmt --check`. `--all-features` compiles integration tests, so per-crate clippy can be green while the gate fails.
- **wasm-gate discipline:** DOM code is `#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]`. Pure logic stays native-testable, or a native `-D warnings` build flags it dead.
- **Attribute URIs verbatim**, kebab-cased as declared. Read-path segments are kebab→camel-cased.
- **Empty fields must be omitted from claims, not sent as `""`** — the extractor drops empty fields, and a rule premise requiring all fields then matches zero rows.
- **Never subscribe to rule-derived conclusions.** Rules are seeded and frozen exactly like views. Read asserted facts only.
- **Crate dependency direction:** `dialog-reactor` depends on `tonk-schema`, never the reverse. So `tonk-schema` CANNOT reach `dialog_reactor::command::{Decode, decode_concept}`. Any test that decodes a command with the real decoder belongs in `dialog-reactor/src/command.rs` (beside `it_decodes_create_space_from_name_only_facts`) or in `tonk-worker` (which depends on both). **Never hand-copy the decoder into a test** — a test that reimplements the decoder passes while the real path breaks, which is the exact silent-failure class this project exists to prevent.

## File Structure

| File | Responsibility |
|---|---|
| `rust/tonk-fab/src/logic.rs` (modify) | Pure, native-testable: claim JSON builders, query body builders, `with=` string construction. All new pure logic lands here beside `pause_claim_json`. |
| `rust/tonk-fab/src/markup.rs` (create) | Pure: builds the FAB's HTML string from `(space_did)`. Native-testable via string assertions. |
| `rust/tonk-fab/src/fab.css` (create) | The ~593-line stylesheet, moved verbatim from `profile.yaml`. `include_str!`-ed. |
| `rust/tonk-fab/src/element.rs` (modify) | DOM: injects markup + once-guarded stylesheet, attaches listeners, dispatches claims. Loses `wrap_telescope_tiles`/`inject_scrim` inference. |
| `rust/tonk-fab/src/space_name.rs` (create) | `<ui-space-name>` — subscribe + inline editable + rename claim. |
| `rust/tonk-fab/src/member_roster.rs` (create) | `<ui-member-roster>` — one directory-mode subscription. |
| `rust/tonk-fab/src/share.rs` (modify) | Rewire both input paths; keep the user-activation clipboard trick. |
| `rust/tonk-fab/src/retry.rs` (create) | Pure: bounded-retry/backoff policy shared by all `ui-` children. |
| `rust/tonk-schema/src/command.rs` (modify) | `RenameRepository` command type; add `space` to `Invite`. |
| `rust/tonk-schema/src/domain.rs` (modify) | `rename_repository::{Name, Subject, RenameRepository}`, `invite::Space` attributes. |
| `rust/tonk-worker/src/router/repository.rs` (modify) | `RenameRepositoryHandler`; teach `InviteHandler` to read `space`. |
| `rust/tonk-worker/src/router/command.rs` (modify) | Register the new handler. |
| `rust/tonk-core/assets/library/core.yaml` (modify) | Delete 3 FAB views. Keep `name-view` and `tonk:view/label`. |
| `rust/tonk-core/assets/library/profile.yaml` (modify) | Delete the FAB view + 3 concepts; rewrite the chrome view's mount. |
| `rust/tonk-fab/src/space_switcher.rs` (create) | `<ui-space-switcher>` — profile-branch space list + per-row name. |
| `rust/tonk-worker/tests/fab_drift.rs` (create) | The load-bearing regression test: hand-built claims agree with the attribute URIs their handlers index on. |

**Task order rationale:** Task 1 (retry policy) and Task 2 (`<ui-space-name>` read) are the spike — they validate the one mechanism with no worked example (a top-level `ui-` element subscribing from Rust-built markup). Tasks 3-4 complete that zone's write path. Only then does the bulk markup move (Tasks 6-8) proceed. **If Task 2 fails, stop and re-plan — every later task assumes it.**

---

### Task 1: Bounded-retry policy

A failed subscription currently schedules an unbounded resubscribe (`ops.rs:551-561`): repo-load failure → `RepositoryNotFound` → 404 → `frame_stream` error → `schedule_resubscribe`. The switcher will spawn one subscribing element per space, so a stale entry becomes a forever-retrying SSE and N stale entries become N of them. Every `ui-` child in this plan needs a give-up story before it ships.

**Files:**
- Create: `rust/tonk-fab/src/retry.rs`
- Modify: `rust/tonk-fab/src/lib.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `pub struct RetryPolicy { attempts: u32 }`, `RetryPolicy::new() -> Self`, `RetryPolicy::next_delay_ms(&mut self) -> Option<i32>` (returns `None` once exhausted → caller renders a terminal state), `RetryPolicy::reset(&mut self)`, `pub const MAX_ATTEMPTS: u32 = 4`.

- [ ] **Step 1: Write the failing test**

Add to `rust/tonk-fab/src/retry.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_backs_off_exponentially() {
        let mut policy = RetryPolicy::new();
        assert_eq!(policy.next_delay_ms(), Some(500));
        assert_eq!(policy.next_delay_ms(), Some(1000));
        assert_eq!(policy.next_delay_ms(), Some(2000));
        assert_eq!(policy.next_delay_ms(), Some(4000));
    }

    #[test]
    fn it_gives_up_after_max_attempts() {
        let mut policy = RetryPolicy::new();
        for _ in 0..MAX_ATTEMPTS {
            assert!(policy.next_delay_ms().is_some());
        }
        assert_eq!(policy.next_delay_ms(), None);
    }

    #[test]
    fn it_restarts_after_reset() {
        let mut policy = RetryPolicy::new();
        while policy.next_delay_ms().is_some() {}
        policy.reset();
        assert_eq!(policy.next_delay_ms(), Some(500));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tonk-fab retry`
Expected: FAIL — `cannot find type RetryPolicy`.

- [ ] **Step 3: Write minimal implementation**

Write `rust/tonk-fab/src/retry.rs` above the test module:

```rust
//! Bounded retry policy for the FAB's subscribing `ui-` children.
//!
//! A subscription that fails (a 404 from an unloadable space, say) is
//! resubscribed by the host without bound. The FAB mounts one subscribing
//! element per listed space, so an unbounded loop per stale entry is real
//! load. These children retry a few times with exponential backoff, then
//! give up and render a terminal state instead of retrying forever.

/// Retries before a subscription is declared dead.
pub const MAX_ATTEMPTS: u32 = 4;

/// Delay before the first retry; each subsequent retry doubles it.
const BASE_DELAY_MS: i32 = 500;

/// Exponential backoff with a hard attempt ceiling.
#[derive(Debug, Default)]
pub struct RetryPolicy {
    attempts: u32,
}

impl RetryPolicy {
    pub fn new() -> Self {
        Self { attempts: 0 }
    }

    /// The next backoff delay, or `None` once the ceiling is reached —
    /// the caller must then stop and render its terminal state.
    pub fn next_delay_ms(&mut self) -> Option<i32> {
        if self.attempts >= MAX_ATTEMPTS {
            return None;
        }
        let delay = BASE_DELAY_MS * (1 << self.attempts);
        self.attempts += 1;
        Some(delay)
    }

    /// Clear the attempt count — call on a frame that arrives successfully.
    pub fn reset(&mut self) {
        self.attempts = 0;
    }
}
```

Add to `rust/tonk-fab/src/lib.rs` after `pub mod logic;`:

```rust
pub mod retry;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p tonk-fab retry`
Expected: PASS, 3 tests.

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-fab/src/retry.rs rust/tonk-fab/src/lib.rs
git commit -m "feat(tonk-fab): bounded retry policy for subscribing chrome"
```

---

### Task 2: `<ui-space-name>` read path (THE SPIKE)

The read direction has no worked example in `tonk-fab` — it issues zero subscriptions today. `<ui-sync-status>` proves the pattern but lives in `tonk-workspace`. This task validates it before anything else depends on it.

The element subscribes to `xyz.tonk.repo/name` on the space's content branch via its own `with="main@{did}"` attribute and plain `consumer::subscribe`. Do **not** use `subscribe_with_route` — it has exactly one caller in the tree (the portal bridge) and is not the precedent.

**Files:**
- Create: `rust/tonk-fab/src/space_name.rs`
- Modify: `rust/tonk-fab/src/logic.rs`, `rust/tonk-fab/src/lib.rs`, `rust/tonk-guest/src/bin/guest.rs`

**Interfaces:**
- Consumes: `RetryPolicy` from Task 1.
- Produces: `logic::repo_name_query_body(subject: &str) -> Result<String, String>` (JSON string; the element parses it), `logic::space_with(did: &str) -> String` (→ `"main@{did}"`), `<ui-space-name space="did:key:...">` registered by `space_name::register()`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `rust/tonk-fab/src/logic.rs`:

```rust
#[test]
fn it_builds_a_with_string_for_a_space_did() {
    assert_eq!(space_with("did:key:z6Mk"), "main@did:key:z6Mk");
}

#[test]
fn it_queries_the_repo_name_by_raw_attribute() {
    let body = repo_name_query_body("did:key:z6Mk").expect("query body builds");
    // The raw attribute URI — NOT a concept name. Nothing seeded is needed,
    // so an old core.yaml cannot break this read.
    assert!(body.contains("xyz.tonk.repo/name"));
    assert!(body.contains("did:key:z6Mk"));
    assert!(!body.contains("tonk:repository"));
}

#[test]
fn it_rejects_an_empty_subject() {
    assert!(repo_name_query_body("").is_err());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tonk-fab logic`
Expected: FAIL — `cannot find function space_with`.

- [ ] **Step 3: Write minimal implementation**

Add to `rust/tonk-fab/src/logic.rs`:

```rust
/// The `with` attribute for a space's content branch: `main@{did}`.
///
/// Each `ui-` child carries its own `with` and subscribes through it —
/// `resolve_with` reads the element's OWN attribute and never walks
/// ancestors, so this must be stamped per element, not inherited.
pub fn space_with(space_did: &str) -> String {
    format!("main@{space_did}")
}

/// The subscribe body for a repository's name.
///
/// An INLINE predicate over the raw `xyz.tonk.repo/name` attribute — it names
/// no concept, so nothing need be seeded on the space's branch and an old
/// `core.yaml` cannot break it. Mirrors `<ui-sync-status>`'s
/// `status_query_body`. `this` is bound to the repo subject by the caller.
pub fn repo_name_query_body(subject: &str) -> Result<String, String> {
    if subject.is_empty() {
        return Err("repo_name_query_body: empty subject".into());
    }
    Ok(json!({
        "predicate": { "with": { "name": {
            "the": "xyz.tonk.repo/name", "as": "String", "cardinality": "one"
        } } },
        "terms": { "this": subject, "name": { "?": { "name": "name" } } }
    })
    .to_string())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p tonk-fab logic`
Expected: PASS.

- [ ] **Step 5: Write the element**

Create `rust/tonk-fab/src/space_name.rs`:

```rust
//! `<ui-space-name>` — a space's repository name, read live from its own branch.
//!
//! Host chrome, NOT space content: it renders the same chip regardless of what
//! the space asserts, so a space choosing wild UI can never redefine or break
//! it — unlike a stdlib `tonk:view/*` view, which lives on the space branch and
//! would need per-space seeding. The `ui-` prefix marks it a host UI primitive,
//! distinct from the `tonk-` data elements.
//!
//! Reads `xyz.tonk.repo/name` through an inline predicate (no concept named,
//! nothing seeded) on its own `with="main@{did}"`, exactly as
//! `<ui-sync-status>` reads sync state.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::JSON;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, HtmlElement, window};

use tonk_host::consumer::{self, Subscription};

use crate::logic::repo_name_query_body;
use crate::retry::RetryPolicy;

/// Shown before the first frame and for a repo with no name — matches the
/// existing "Untitled" fallback the seeded view rendered.
const UNTITLED: &str = "Untitled";

const SUB_TAG: &str = "ui-space-name";

#[derive(Default)]
pub struct UiSpaceNameElement {
    subscription: Rc<RefCell<Option<Subscription>>>,
    retry: Rc<RefCell<RetryPolicy>>,
}

impl CustomElement for UiSpaceNameElement {
    fn inject_children(&mut self, this: &HtmlElement) {
        this.set_text_content(Some(UNTITLED));
    }

    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["space"]
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        let Some(space) = this.get_attribute("space").filter(|s| !s.is_empty()) else {
            // No space yet (an unsubstituted `{id}` placeholder, say) — the
            // attribute callback re-runs this when it lands.
            return;
        };
        // Stamp our own routing context: `resolve_with` reads THIS element's
        // attribute and never walks ancestors.
        let _ = this.set_attribute("with", &crate::logic::space_with(&space));

        let subscription = self.subscription.clone();
        let retry = self.retry.clone();
        let host = this.clone();
        spawn_local(async move {
            if !host.is_connected() || subscription.borrow().is_some() {
                return;
            }
            subscribe_name(&host, &space, subscription, retry);
        });
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.subscription.borrow_mut().take();
    }
}

fn subscribe_name(
    host: &HtmlElement,
    space: &str,
    subscription: Rc<RefCell<Option<Subscription>>>,
    retry: Rc<RefCell<RetryPolicy>>,
) {
    let body = match repo_name_query_body(space) {
        Ok(body) => body,
        Err(err) => {
            tonk_common::log!("ui-space-name: query build failed: {err}");
            return;
        }
    };
    let Ok(parsed) = JSON::parse(&body) else {
        tonk_common::log!("ui-space-name: query JSON parse failed");
        return;
    };
    let consumer_el: Element = host.clone().into();
    let tag = JsValue::from_str(SUB_TAG);
    match consumer::subscribe(&consumer_el, &parsed, Some(&tag)) {
        Ok(sub) => {
            retry.borrow_mut().reset();
            *subscription.borrow_mut() = Some(sub);
        }
        Err(err) => {
            // Bounded, unlike the host's default resubscribe loop.
            let delay = retry.borrow_mut().next_delay_ms();
            match delay {
                Some(_) => tonk_common::log!("ui-space-name: subscribe failed, will retry: {err:?}"),
                None => {
                    tonk_common::log!("ui-space-name: subscribe failed, giving up: {err:?}");
                    let _ = host.set_attribute("data-state", "unavailable");
                }
            }
        }
    }
}

/// Register `<ui-space-name>`. Idempotent.
pub fn register() {
    let registered = window()
        .map(|win| !win.custom_elements().get("ui-space-name").is_undefined())
        .unwrap_or(false);
    if registered {
        return;
    }
    UiSpaceNameElement::define("ui-space-name");
}
```

Add to `rust/tonk-fab/src/lib.rs`:

```rust
#[cfg(target_arch = "wasm32")]
mod space_name;
```

and inside `register()`:

```rust
space_name::register();
```

- [ ] **Step 6: Verify it compiles for both targets**

Run: `cargo clippy -p tonk-fab --all-targets --all-features`
Expected: clean. The native build must not flag `space_name` dead — it is `#[cfg(target_arch = "wasm32")]`.

- [ ] **Step 7: Verify the spike in a browser**

Mount `<ui-space-name space="{a real space DID}">` inside the existing FAB view temporarily and confirm it renders the live name and updates on rename from another surface.

Run: `/run` (or the project's app-launch skill) and inspect. This is the gate — **a top-level `ui-` element subscribing from Rust has no precedent.** If the frame never arrives, stop and re-plan before Task 3.

- [ ] **Step 8: Commit**

```bash
git add rust/tonk-fab/src/space_name.rs rust/tonk-fab/src/logic.rs rust/tonk-fab/src/lib.rs
git commit -m "feat(tonk-fab): read space name from its own branch, unseeded"
```

---

### Task 3: `RenameRepository` command + handler

Rename is a declarative rule on the *space* branch (`core.yaml:825`) — a profile-dispatched claim has no rule there to consume it. It needs a worker handler shaped like `PauseSyncHandler`. This is the one place the design changes semantics.

**Files:**
- Modify: `rust/tonk-schema/src/domain.rs`, `rust/tonk-schema/src/command.rs`, `rust/tonk-worker/src/router/repository.rs`, `rust/tonk-worker/src/router/command.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `tonk_schema::command::RenameRepository { this, name, space, marker }`, `tonk_schema::domain::command::rename_repository::{Value, Space, Rename}`, `repository::{RenameRepositoryHandler::new(), RenameOutcome, rename_outcome}`.

- [ ] **Step 1: Write the failing test**

Add to **`rust/dialog-reactor/src/command.rs`**'s test module — NOT `tonk-schema`. `dialog-reactor` depends on `tonk-schema`, so only it can decode these commands with the real `decode_concept`. Put this beside its siblings `it_decodes_create_space_from_name_only_facts` (~line 491) and `it_decodes_remove_space_from_a_data_remove_fact` (~line 510), and follow their exact shape (`entity(...)`, `Changes::new()`, `the!(...)`, `facts_for(changes)`, `Type::decode(this, &facts)`):

```rust
#[dialog_common::test]
fn it_decodes_rename_repository_naming_its_target_space() {
    // The handler must read the target space off the COMMAND, not the
    // dispatch origin — that is what lets the FAB dispatch from the profile
    // branch with nothing seeded per-space.
    let command = RenameRepository {
        this: "cmd:1".parse().unwrap(),
        name: rename_repository::Value("Renamed".into()),
        space: rename_repository::Space("did:key:z6Mk".parse().unwrap()),
        marker: rename_repository::Rename("tonk:repository".parse().unwrap()),
    };
    let facts = command.encode();
    let decoded = RenameRepository::decode("cmd:1".parse().unwrap(), &facts)
        .expect("decodes from its own facts");
    assert_eq!(decoded.space.0.to_string(), "did:key:z6Mk");
    assert_eq!(decoded.name.0, "Renamed");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tonk-schema it_decodes_rename_repository`
Expected: FAIL — `cannot find type RenameRepository`.

- [ ] **Step 3: Add the domain attributes**

Add to `rust/tonk-schema/src/domain.rs` inside the `command` module, after `pub mod pause_sync { .. }`:

```rust
/// Attributes the `tonk/rename-repository` command carries when the FAB
/// dispatches it from the PROFILE branch.
pub mod rename_repository {
    use super::super::Entity;
    use super::Attribute;

    /// The new repository name, read from the chip's `<tonk-editable>` on
    /// commit. The derived attribute is
    /// `dom.event.current-target/value`.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("dom.event.current-target")]
    pub struct Value(pub String);

    /// The target space DID — the repository to rename. Read by the handler
    /// in place of the dispatch origin, so the command can be defined and
    /// dispatched on the PROFILE branch and the FAB depends on nothing
    /// seeded per-space. Mirrors `pause_sync::Space`. The derived attribute
    /// is `xyz.tonk.rename-repository/space`.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.rename-repository")]
    pub struct Space(pub Entity);

    /// Per-command marker keeping this shape distinct from `profile/rename`,
    /// which also reads `dom.event.current-target/value`. The derived
    /// attribute is `dom.event.current-target.dataset/rename`.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("dom.event.current-target.dataset")]
    pub struct Rename(pub Entity);
}
```

- [ ] **Step 4: Add the command type**

Add to `rust/tonk-schema/src/command.rs`, after `PauseSync`:

```rust
/// Rename a space's repository from the FAB.
///
/// The space-side `tonk/rename-repository` rule (`core.yaml`) cannot consume a
/// claim dispatched on the profile branch, so this carries its target `space`
/// and is executed by a worker handler instead — the `PauseSync` pattern. That
/// is what lets the FAB's name chip depend on nothing seeded per-space.
#[derive(Concept, Debug, Clone, PartialEq, PartialOrd)]
pub struct RenameRepository {
    /// The command entity (a fresh id per commit).
    pub this: Entity,
    /// The new name, read from the editable's value on commit.
    pub name: crate::domain::command::rename_repository::Value,
    /// The target space DID — the repository to rename.
    pub space: crate::domain::command::rename_repository::Space,
    /// Per-command marker distinguishing this from `profile/rename`, which
    /// shares the `{this, value}` shape.
    pub marker: crate::domain::command::rename_repository::Rename,
}

impl Command for RenameRepository {
    type Input = Self;
    type Output = ();
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p tonk-schema it_decodes_rename_repository`
Expected: PASS.

- [ ] **Step 6: Write the outcome-mapping test**

The handler itself is wasm-gated, so the testable seam is the pure mapping
from a repository result to an outcome the UI can reflect. This pins the
design decision that a missing replica must NOT be swallowed.

Add to `rust/tonk-worker/src/router/repository.rs`'s test module:

```rust
#[dialog_common::test]
fn it_maps_a_failed_rename_to_failed_rather_than_success() {
    // PauseSyncHandler logs and returns on a missing replica. Rename must
    // not: a silently-dropped rename looks successful to the user, which is
    // the exact failure class this design attacks.
    // `RepositoryError` has no NotFound variant — an absent replica surfaces
    // as Internal from the acquire.
    let outcome = rename_outcome(Err(RepositoryError::Internal("no such replica".into())));
    assert_eq!(outcome, RenameOutcome::Failed);
}

#[dialog_common::test]
fn it_maps_a_successful_rename_to_renamed() {
    assert_eq!(rename_outcome(Ok(())), RenameOutcome::Renamed);
}
```

- [ ] **Step 7: Implement the handler**

Add to `rust/tonk-worker/src/router/repository.rs`, modelled on `PauseSyncHandler` (`:926-996`):

```rust
/// Outcome of a rename, surfaced rather than swallowed.
///
/// `PauseSyncHandler` logs and returns on a missing replica. Rename must not:
/// a silently-dropped rename looks successful to the user, which is the
/// failure class this whole design attacks.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RenameOutcome {
    Renamed,
    Failed,
}

/// Map a rename result to an outcome the chip can reflect.
///
/// Pure and native-testable — the handler around it is wasm-gated, so this is
/// the seam where the "do not swallow a failed rename" decision is pinned.
/// Any error is `Failed`: `RepositoryError` carries no NotFound variant, so an
/// absent replica arrives as `Internal` from the acquire, and the chip's
/// response is the same either way — revert, do not show a phantom success.
pub(crate) fn rename_outcome(result: Result<(), RepositoryError>) -> RenameOutcome {
    match result {
        Ok(()) => RenameOutcome::Renamed,
        Err(_) => RenameOutcome::Failed,
    }
}

pub(crate) struct RenameRepositoryHandler {
    attributes: Vec<String>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl RenameRepositoryHandler {
    pub(crate) fn new() -> Self {
        use crate::reactor::Decode as _;
        Self {
            attributes: tonk_schema::command::RenameRepository::trigger_attributes(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for RenameRepositoryHandler {
    fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        use crate::reactor::Decode as _;
        facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|this| tonk_schema::command::RenameRepository::decode(this, facts))
            .is_some()
    }

    fn run(
        &self,
        facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        use crate::reactor::Decode as _;
        use tonk_schema::prelude::DidExt as _;

        // Decode synchronously to read the target space off the command — the
        // handler renames THAT repository, not the dispatch origin's, so the
        // command can be dispatched from the profile branch.
        let decoded = facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|entity| tonk_schema::command::RenameRepository::decode(entity, facts));
        let env = env.clone();

        Box::pin(async move {
            let Some(command) = decoded else { return };
            let Ok(did) = command.space.0.to_string().parse::<dialog_varsig::Did>() else {
                log!("RenameRepository: unparseable target space, skipping");
                return;
            };
            // `repo_key()` is the FULL DID, not a suffix.
            let repo = did.repo_key().to_owned();
            log!("command RenameRepository repo={}", repo);

            if let Err(error) = run_rename_repository(&env, &repo, &command.name.0).await {
                log!("RenameRepository for repo '{}' failed: {}", repo, error);
            }
        })
    }
}
```

Implement `run_rename_repository(env, repo, name)` beside `run_pause_sync`, asserting `tonk_schema::domain::repo::Name(name.into())` on the repo's `CONTENT_BRANCH` keyed by its subject entity — the same fact the space-side rule wrote.

- [ ] **Step 8: Register the handler**

Add to `rust/tonk-worker/src/router/command.rs` beside line 123:

```rust
registry.register(Box::new(super::repository::RenameRepositoryHandler::new()));
```

- [ ] **Step 9: Run tests**

Run: `cargo test -p tonk-worker rename && cargo test -p tonk-schema rename`
Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add rust/tonk-schema/src rust/tonk-worker/src/router
git commit -m "feat(tonk-worker): rename a repository by target space, not origin"
```

---

### Task 4: `<ui-space-name>` write path

**Files:**
- Modify: `rust/tonk-fab/src/logic.rs`, `rust/tonk-fab/src/space_name.rs`

**Interfaces:**
- Consumes: `RenameRepository`'s attribute URIs from Task 3; `<ui-space-name>` from Task 2.
- Produces: `logic::rename_repo_claim_json(space: &str, name: &str) -> Value`.

- [ ] **Step 1: Write the failing test**

Add to `rust/tonk-fab/src/logic.rs`'s test module:

```rust
#[test]
fn it_inlines_the_rename_descriptor_and_names_its_target_space() {
    let claim = rename_repo_claim_json("did:key:z6Mk", "Renamed");
    let text = claim.to_string();
    // The descriptor rides WITH the claim — nothing seeded is consulted.
    assert!(text.contains("xyz.tonk.rename-repository/space"));
    assert!(text.contains("dom.event.current-target/value"));
    assert!(text.contains("did:key:z6Mk"));
    assert!(text.contains("Renamed"));
}

#[test]
fn it_omits_an_empty_name_rather_than_sending_a_blank() {
    // The extractor drops empty fields; a blank would store no fact and the
    // handler would never fire.
    let claim = rename_repo_claim_json("did:key:z6Mk", "");
    assert!(!claim.to_string().contains("\"value\""));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tonk-fab it_inlines_the_rename`
Expected: FAIL — `cannot find function rename_repo_claim_json`.

- [ ] **Step 3: Implement**

Add to `rust/tonk-fab/src/logic.rs`, modelled on `pause_claim_json` (`:281`):

```rust
/// Build a `TransactRequest` body for `tonk/rename-repository`.
///
/// A transient carrying the target `space` and the new `value`. Dispatched
/// routeless via `window.tonk.transact`, so it lands on the FAB's own
/// `main@profile:tonk`; the worker's handler reads `space` to rename that
/// repository — nothing space-side is required. `this` is omitted so the
/// worker mints it from `(descriptor, parameters)`.
///
/// An empty `name` is omitted entirely: the extractor drops empty fields, so
/// a blank would store no fact and the command would never fire.
pub fn rename_repo_claim_json(space: &str, name: &str) -> Value {
    let mut parameters = json!({
        "space": space,
        "rename": "tonk:repository"
    });
    if !name.is_empty() {
        parameters["value"] = json!(name);
    }
    json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "Rename a space's repository from the FAB.",
                        "with": {
                            "value":  { "the": "dom.event.current-target/value", "as": "String" },
                            "space":  { "the": "xyz.tonk.rename-repository/space", "as": "Entity" },
                            "rename": { "the": "dom.event.current-target.dataset/rename", "as": "Entity" }
                        }
                    }
                },
                "parameters": parameters
            }
        }]
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p tonk-fab logic`
Expected: PASS.

- [ ] **Step 5: Wire the editable**

In `space_name.rs`, render a `<tonk-editable value=… data-rename="tonk:repository">` child and attach a `change` listener in Rust — the tonk-display delegate that used to resolve `onchange=` is gone. On commit, build the claim and dispatch it via `window.tonk.transact` exactly as `element.rs:344-351` does for pause. Reflect `RenameOutcome::NotFound` by restoring the previous name rather than optimistically keeping the typed one.

- [ ] **Step 6: Verify in a browser**

Rename a space from the FAB; confirm the name persists and propagates. Then rename a space whose replica is absent and confirm the chip reverts rather than showing a phantom success.

- [ ] **Step 7: Commit**

```bash
git add rust/tonk-fab/src
git commit -m "feat(tonk-fab): rename the space from the name chip"
```

---

### Task 5: `<ui-member-roster>`

One directory-mode subscription with three fields on the same entity — **not** three sync-status-shaped subscriptions, which would need client-side row-joining no existing element does.

**Files:**
- Create: `rust/tonk-fab/src/member_roster.rs`
- Modify: `rust/tonk-fab/src/logic.rs`, `rust/tonk-fab/src/lib.rs`

**Interfaces:**
- Consumes: `RetryPolicy` (Task 1), `logic::space_with` (Task 2).
- Produces: `logic::member_roster_query_body() -> String`, `<ui-member-roster space="did:key:…">` via `member_roster::register()`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn it_queries_all_member_fields_in_one_directory_predicate() {
    let body = member_roster_query_body();
    assert!(body.contains("xyz.tonk.membership/name"));
    assert!(body.contains("xyz.tonk.membership/member"));
    assert!(body.contains("xyz.tonk.membership/role"));
    // Directory mode: `this` is an unbound variable, so every member row
    // comes back. A bound `this` would return one.
    assert!(body.contains("\"this\": { \"?\""));
    // No concept named — nothing seeded is consulted.
    assert!(!body.contains("tonk:member"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tonk-fab member_roster`
Expected: FAIL — `cannot find function member_roster_query_body`.

- [ ] **Step 3: Implement**

```rust
/// The subscribe body for a space's member roster.
///
/// ONE inline predicate carrying all three fields on the same entity, in
/// directory mode (`this` unbound), so each member returns as a row. Three
/// separate subscriptions would need client-side row-joining that no existing
/// element does.
///
/// All three are required fields: a member missing a synced name or role is
/// invisible. That matches the seeded view's behaviour, but it is now this
/// element's choice.
pub fn member_roster_query_body() -> String {
    json!({
        "predicate": { "with": {
            "member": { "the": "xyz.tonk.membership/member", "as": "Entity", "cardinality": "one" },
            "role":   { "the": "xyz.tonk.membership/role",   "as": "Entity", "cardinality": "one" },
            "name":   { "the": "xyz.tonk.membership/name",   "as": "String", "cardinality": "one" }
        } },
        "terms": {
            "this":   { "?": { "name": "this" } },
            "member": { "?": { "name": "member" } },
            "role":   { "?": { "name": "role" } },
            "name":   { "?": { "name": "name" } }
        }
    })
    .to_string()
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p tonk-fab member_roster`
Expected: PASS.

- [ ] **Step 5: Extract the shared subscribing scaffolding**

`<ui-space-name>` (Task 2) and this element share five things: `shadow() -> false`, an observed `space` attribute, stamping their own `with=`, plain `consumer::subscribe`, and `RetryPolicy` on failure. They differ only in the query body and how they render a frame.

This is the second of three (`<ui-space-switcher>` in Task 5b is the third), so extract now rather than duplicating twice more. Create `rust/tonk-fab/src/subscribing.rs` holding the common scaffolding, and refactor `space_name.rs` onto it in the same commit — its tests from Task 2 must still pass unchanged, which is what proves the refactor safe.

Suggested seam (adjust to what the two elements actually share once written):

```rust
/// The per-element behaviour a subscribing `ui-` child supplies; the
/// scaffolding around it (with-stamping, subscribe, retry, teardown) is
/// shared.
pub trait Subscribing {
    /// The subscribe body, built from the element's `space` attribute.
    fn query_body(&self, space: &str) -> Result<String, String>;
    /// Render one subscription frame into the host.
    fn render(&self, host: &HtmlElement, payload: JsValue);
    /// Tag distinguishing this element's subscription.
    fn tag(&self) -> &'static str;
}
```

- [ ] **Step 6: Write the element**

Create `rust/tonk-fab/src/member_roster.rs` on that scaffolding. On each frame, rebuild one `<span class="fab__menu-item fab__menu-item--member">{name}</span>` per conclusion — the markup the deleted `fab-roster` view used to supply.

Register in `lib.rs`.

- [ ] **Step 7: Verify the refactor preserved Task 2's behaviour**

Run: `cargo test -p tonk-fab`
Expected: PASS — including every `space_name` / `logic` test from Task 2, unchanged.

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-fab/src/member_roster.rs rust/tonk-fab/src/logic.rs rust/tonk-fab/src/lib.rs
git commit -m "feat(tonk-fab): render the member roster from unseeded facts"
```

---

### Task 5b: `<ui-space-switcher>`

The switcher reads the PROFILE branch (`tonk:space` records), not a space — so
it is safe from seed drift today. It still moves, because the FAB now owns its
markup and `view=tonk:view/fab-menu` is being deleted with the rest.

The seeded view did filtering the Rust must reproduce: hide the profile's own
self-replica (`[data-kind="tonk:profile"]`), reflect seeding state
(`[data-status="tonk:blank"]`), and exclude the active space
(`<ui-dropdown exclude=>`).

Each row shows the space's *own* repo name via `<ui-space-name>`, not the
profile-side `xyz.tonk.replica/name`. That costs one subscription per row, and
it is deliberate: the space's own name is the cross-device source of truth
(`profile.yaml:186-190` states this for the Hub) and the replica name goes
stale. This is the trade the spec names.

**Files:**
- Create: `rust/tonk-fab/src/space_switcher.rs`
- Modify: `rust/tonk-fab/src/logic.rs`, `rust/tonk-fab/src/lib.rs`

**Interfaces:**
- Consumes: `RetryPolicy` (Task 1), `<ui-space-name>` (Task 2), the `Subscribing` scaffolding extracted in Task 5.
- Produces: `logic::space_list_query_body() -> String`, `<ui-space-switcher exclude="did:key:…">` via `space_switcher::register()`.

Build this on Task 5's `subscribing.rs` scaffolding — it is the third of the three subscribing elements, so it should need only a query body and a render.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn it_queries_the_profile_space_list_by_raw_attribute() {
    let body = space_list_query_body();
    assert!(body.contains("xyz.tonk.replica/subject"));
    assert!(body.contains("xyz.tonk.replica/kind"));
    assert!(body.contains("xyz.tonk.replica/status"));
    // Directory mode over every replica record.
    assert!(body.contains("\"this\": { \"?\""));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tonk-fab space_list`
Expected: FAIL — `cannot find function space_list_query_body`.

- [ ] **Step 3: Implement**

```rust
/// The subscribe body for the profile's space list.
///
/// Reads the PROFILE branch's replica records by raw attribute. `name` is
/// deliberately absent: each row renders the space's OWN repo name via
/// `<ui-space-name>`, since the profile-side replica name goes stale.
pub fn space_list_query_body() -> String {
    json!({
        "predicate": { "with": {
            "subject": { "the": "xyz.tonk.replica/subject", "as": "Entity", "cardinality": "one" },
            "kind":    { "the": "xyz.tonk.replica/kind",    "as": "Entity", "cardinality": "one" },
            "status":  { "the": "xyz.tonk.replica/status",  "as": "Entity", "cardinality": "one" }
        } },
        "terms": {
            "this":    { "?": { "name": "this" } },
            "subject": { "?": { "name": "subject" } },
            "kind":    { "?": { "name": "kind" } },
            "status":  { "?": { "name": "status" } }
        }
    })
    .to_string()
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p tonk-fab space_list`
Expected: PASS.

- [ ] **Step 5: Write the element**

Create `rust/tonk-fab/src/space_switcher.rs` on Task 5's `subscribing.rs`
scaffolding. Note it carries `with="main@profile:tonk"` (not a space `with`) —
if the scaffolding assumes a space-derived `with`, this is the element that
proves the seam needs to accept either. Per frame, render one
`<a class="fab__menu-item" href="/space/{subject}">` per row, each containing
`<ui-space-name space="{subject}">`. Skip rows where `kind == "tonk:profile"`
or `subject == exclude`; stamp `data-status` so the existing CSS dims seeding
spots. Append the static "all spots" and "new" action items.

Register in `lib.rs`.

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-fab/src/space_switcher.rs rust/tonk-fab/src/logic.rs rust/tonk-fab/src/lib.rs
git commit -m "feat(tonk-fab): own the space switcher"
```

---

### Task 6: Move the stylesheet

`<tonk-fab>` has no shadow root (`element.rs:33`, `shadow() -> false`) — the whole chain is light DOM, so the stylesheet needs an explicit mechanism. `include_str!` + once-guarded head injection keeps the component self-contained and lets the CSS be diffed beside the markup that uses it. It must be guarded: the element re-binds on clone (`__tonkFabBound` is dropped by `cloneNode`, `element.rs:100-113`) and tonk-display clones the chrome view.

**Files:**
- Create: `rust/tonk-fab/src/fab.css`
- Modify: `rust/tonk-fab/src/element.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `element::ensure_stylesheet()` — idempotent, keyed by element id `tonk-fab-styles`.

- [ ] **Step 1: Extract the CSS verbatim**

Copy the contents of the `<style>` block inside `profile.yaml`'s FAB view (`profile.yaml:812-1688`, the stylesheet portion) into `rust/tonk-fab/src/fab.css`. **Verbatim** — no re-indentation or "tidying". Any change here is a visual regression with no test to catch it.

- [ ] **Step 2: Write the failing test**

```rust
#[test]
fn it_ships_the_stylesheet_with_the_crate() {
    let css = include_str!("fab.css");
    // A representative selector from each zone, so a truncated copy fails.
    assert!(css.contains(".fab__cap-l"));
    assert!(css.contains(".fab__menu-item"));
    assert!(css.contains(".fab__share-label"));
    assert!(css.contains(".wizard__card"));
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p tonk-fab it_ships_the_stylesheet`
Expected: FAIL — `couldn't read fab.css` (before Step 1) or a missing selector (if the copy is short).

- [ ] **Step 4: Implement injection**

Add to `rust/tonk-fab/src/element.rs`:

```rust
/// The id of the injected stylesheet, so injection is idempotent.
const STYLE_ID: &str = "tonk-fab-styles";

/// Inject the FAB stylesheet once per document.
///
/// The element has no shadow root, so the CSS is global. It must be guarded:
/// the element re-binds on every clone (tonk-display clones the chrome view),
/// and an unguarded injection would append a copy per mount.
fn ensure_stylesheet() {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    if document.get_element_by_id(STYLE_ID).is_some() {
        return;
    }
    let Ok(style) = document.create_element("style") else {
        return;
    };
    let _ = style.set_attribute("id", STYLE_ID);
    style.set_text_content(Some(include_str!("fab.css")));
    if let Some(head) = document.head() {
        let _ = head.append_child(style.as_ref());
    }
}
```

Call `ensure_stylesheet()` at the top of `connected_callback`, **outside** the `__tonkFabBound` guard — a clone in a fresh document still needs it.

- [ ] **Step 5: Run tests**

Run: `cargo test -p tonk-fab && cargo clippy -p tonk-fab --all-targets --all-features`
Expected: PASS, clean.

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-fab/src/fab.css rust/tonk-fab/src/element.rs
git commit -m "feat(tonk-fab): ship the stylesheet with the crate"
```

---

### Task 7: Move the markup

The largest task. The wizard alone means CSS-radio paging (`#wiz-start`/`#wiz-template`), the hidden `Untitled` sentinel (non-empty, or the field is dropped and the command never fires), four template radios, the submit-inside-`<label>` trick, and `<tonk-default-remote field="remote" auto>`.

Owning the markup also lets `inject_scrim` and `wrap_telescope_tiles` (`element.rs:196-208`) go: they exist only to retrofit structure onto view-supplied markup — inferring the cap from `.fab` child 0 and forcing the scrim to be a sibling because the view renderer drops empty elements. Emit the wrappers and scrim directly instead.

**Files:**
- Create: `rust/tonk-fab/src/markup.rs`
- Modify: `rust/tonk-fab/src/element.rs`, `rust/tonk-fab/src/lib.rs`

**Interfaces:**
- Consumes: `<ui-space-name>` (Task 2), `<ui-member-roster>` (Task 5), `<ui-space-switcher>` (Task 5b).
- Produces: `markup::fab_html(space_did: &str) -> String`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn it_stamps_the_space_onto_every_cross_branch_child() {
    let html = fab_html("did:key:z6Mk");
    // Each ui- child must carry its OWN space: resolve_with reads the
    // element's own attribute and never walks ancestors.
    assert!(html.contains(r#"<ui-space-name space="did:key:z6Mk""#));
    assert!(html.contains(r#"<ui-member-roster space="did:key:z6Mk""#));
    assert!(html.contains(r#"<ui-sync-status with="main@did:key:z6Mk""#));
}

#[test]
fn it_emits_telescope_wrappers_directly() {
    let html = fab_html("did:key:z6Mk");
    // The element owns its structure now, so the wrappers are authored, not
    // inferred from child order.
    assert!(html.contains("fab__tele"));
    assert!(html.contains("fab__scrim"));
}

#[test]
fn it_carries_the_untitled_sentinel_in_the_wizard() {
    let html = fab_html("did:key:z6Mk");
    // Must be non-empty: the extractor omits blank fields, and with no
    // `name` fact the transient never triggers the handler.
    assert!(html.contains(r#"<input type="hidden" name="name" value="Untitled">"#));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p tonk-fab markup`
Expected: FAIL — `cannot find function fab_html`.

- [ ] **Step 3: Implement**

Create `rust/tonk-fab/src/markup.rs` with `pub fn fab_html(space_did: &str) -> String`, porting the markup from `profile.yaml:818-1090` (bar + dialog). Substitute `{dom.host/data-space}` with `space_did` — the FAB now performs the substitution tonk-display's template engine used to do. Emit `.fab__tele` wrappers and the `.fab__scrim` sibling directly.

- [ ] **Step 4: Run tests**

Run: `cargo test -p tonk-fab markup`
Expected: PASS.

- [ ] **Step 5: Wire into the element**

In `element.rs`, implement `inject_children` to set `fab_html(self.space)` as `innerHTML`. Delete `inject_scrim` and `wrap_telescope_tiles` and their calls. Keep `attach_drag`, `attach_gestures`, `preload_menu_widths`, `restore_position` and the `__tonkFabBound` guard.

- [ ] **Step 6: Commit**

```bash
git add rust/tonk-fab/src/markup.rs rust/tonk-fab/src/element.rs rust/tonk-fab/src/lib.rs
git commit -m "feat(tonk-fab): own the FAB markup"
```

---

### Task 8: Rust-side command dispatch for wizard, profile rename, share

The delegate is gone, and building the claim is the easy part. These are its side effects (`delegate.rs:148-175`), all load-bearing:

- **`prevent-default`** — without it the create form does a native submit and reloads with `?name=` (`extract.rs:631-635`).
- **`data-close-dialog`** — sets the `<wa-dialog>`'s `open = false`. The create form carries it (`profile.yaml:942`); without it the wizard never closes after Create.
- **`data-close-radio`** — re-checks the paging radio and calls `form.reset()`.
- **`<tonk-editable>` commit wiring** — Rust must attach the `change` listeners.

`<tonk-share>` needs both its inputs rewired: it currently gets the URL from a `tonk-display:result` fired by the `fab-invite` child, and mints by letting the click fall through to the child form's `onsubmit`. Both children are gone. The `ClipboardItem`-promise trick (`share.rs:8-22`) must survive — the clipboard write opens synchronously inside the click's user activation and resolves when the mint lands, so the mint claim must dispatch inside that same handler.

**Files:**
- Modify: `rust/tonk-fab/src/element.rs`, `rust/tonk-fab/src/share.rs`, `rust/tonk-fab/src/logic.rs`
- Modify: `rust/tonk-schema/src/command.rs`, `rust/tonk-worker/src/router/repository.rs`

**Interfaces:**
- Consumes: `markup::fab_html` (Task 7).
- Produces: `logic::create_space_claim_json(name, remote, template) -> Value`, `logic::profile_rename_claim_json(name) -> Value`, `logic::invite_claim_json(space, time) -> Value`, `logic::invite_link_query_body(subject) -> String`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn it_uses_the_declared_form_attribute_uris_for_create_space() {
    let claim = create_space_claim_json("Untitled", "https://x", "wiki");
    let text = claim.to_string();
    // Verbatim, kebab-cased as declared — the handler matches on these.
    assert!(text.contains("dom.event.current-target.elements.name/value"));
    assert!(text.contains("dom.event.current-target.elements.remote/value"));
    assert!(text.contains("dom.event.current-target.elements.template/value"));
}

#[test]
fn it_omits_a_blank_remote_rather_than_sending_an_empty_string() {
    let claim = create_space_claim_json("Untitled", "", "blank");
    assert!(!claim.to_string().contains("\"remote\""));
}

#[test]
fn it_names_the_target_space_on_the_invite() {
    let claim = invite_claim_json("did:key:z6Mk", 1.0);
    assert!(claim.to_string().contains("xyz.tonk.invite/space"));
    assert!(claim.to_string().contains("did:key:z6Mk"));
}

#[test]
fn it_reads_the_invite_link_not_the_rule_derived_agent_invite() {
    let body = invite_link_query_body("did:key:z6Mk");
    // `tonk:agent-invite` is rule-derived; rules are frozen like views.
    assert!(body.contains("xyz.tonk.credential/link"));
    assert!(!body.contains("agent-invite"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tonk-fab logic`
Expected: FAIL — missing functions.

- [ ] **Step 3: Add `space` to `Invite`**

`InviteHandler` reads `env.origin().repo` (`repository.rs:650`), which is empty for a routeless profile-branch dispatch. Add a `space` field mirroring `PauseSync::space` and read it in place of the origin. `RemoveSpaceHandler` already demonstrates the empty-origin property. Add `domain::command::invite::Space` with `#[domain("xyz.tonk.invite")]`.

- [ ] **Step 4: Implement the claim builders**

Add all four to `logic.rs`, following `pause_claim_json`'s shape: inline descriptor, omit empty fields, `this` omitted so the worker mints it.

- [ ] **Step 5: Attach the listeners**

In `element.rs`, on the create form's `submit`: call `preventDefault()`, read the fields, dispatch `create_space_claim_json`, set the `<wa-dialog>`'s `open = false`, re-check the paging radio, and `form.reset()`.

On the profile-name `<tonk-editable>`'s `change`: dispatch `profile_rename_claim_json`.

- [ ] **Step 6: Rewire `<tonk-share>`**

Replace the `tonk-display:result` listener with a subscription to `xyz.tonk.credential/link`. Dispatch `invite_claim_json` inside the click handler that opens the clipboard promise — the write must stay inside the user activation.

Note in a comment that the link is overlay-only and session-scoped (`.overlay().write()`, never replicated — `repository.rs:768-777`), so it resolves only in the minting session. `Credential` is cardinality-one on the repo subject, so a re-mint supersedes in place.

- [ ] **Step 7: Verify every interaction in a browser**

Create a space (wizard closes, space appears, navigates in). Rename the profile. Share (one click → link on the clipboard). Alt-click the disc (pause toggles). Drag the FAB (dock persists).

- [ ] **Step 8: Commit**

```bash
git add rust/tonk-fab/src rust/tonk-schema/src rust/tonk-worker/src
git commit -m "feat(tonk-fab): dispatch every FAB command from rust"
```

---

### Task 9: Delete the seeded views and rewire the mount

**Files:**
- Modify: `rust/tonk-core/assets/library/core.yaml`, `rust/tonk-core/assets/library/profile.yaml`
- Modify: `rust/tonk-portal/src/lib.rs`; Delete: `rust/tonk-portal/src/fab.rs`

- [ ] **Step 1: Rewrite the chrome view**

Replace `profile.yaml:2071-2076`'s view body with:

```yaml
  display: |
    <tonk-site with="main@{id}" allow="main@{id}" path={rest}></tonk-site>
    <tonk-fab with="main@profile:tonk" space="{id}"></tonk-fab>
```

The view keeps its stable `this:`, so the re-seed replaces the mount in place — no double-FAB on existing profiles.

- [ ] **Step 2: Delete from `core.yaml`**

Remove `tonk:view/fab-roster` (`:670`, `:691`), `tonk:view/fab-invite` (`:708`, `:727`), `tonk:repository/fab-share` (`:755`).

**Keep `tonk:repository/name-view` (`:844`)** — it is the default entity view for `tonk:repository` and the workspace topbar's title chip. Only the FAB's implicit no-`view=` mount of it goes.
**Keep `tonk:view/label` (`:903`)** — used by the Hub cards (`profile.yaml:511`), the Hub's delete-confirm overlay (`:526`) and `wiki.yaml:624`.

- [ ] **Step 3: Delete from `profile.yaml`**

Remove the FAB view (`:812-1688`), `tonk:profile/fab` (`:677`), `tonk:view/fab-menu` (`:699`, `:718`), and the `tonk:profile/name` view (`:795`).

- [ ] **Step 4: Delete the dead portal**

`<tonk-fab-portal>` (`rust/tonk-portal/src/fab.rs`, ~350 lines) is exported at `lib.rs:43` but called from nowhere and appears in no view. Remove the file and the export.

- [ ] **Step 5: Verify no `fab__` references remain on the space branch**

Run: `grep -c "fab__" rust/tonk-core/assets/library/core.yaml`
Expected: `0`.

- [ ] **Step 6: Run the library guard tests**

Run: `cargo test -p tonk-worker --test standard_library`
Expected: PASS — every asset still parses, analyzes and lowers.

- [ ] **Step 7: Commit**

```bash
git add rust/tonk-core/assets/library rust/tonk-portal/src
git commit -m "refactor(tonk-core): stop seeding FAB chrome into spaces"
```

---

### Task 10: The drift regression test

The property the whole design buys, which nothing currently tests. Without it, a future change can silently reintroduce the drift.

There is deliberately **no old-`core.yaml` fixture here.** The whole point of the design is that the FAB consults nothing seeded — it reads raw attribute URIs and inlines its own descriptors — so no native test needs an old library to prove it. Seeding a fixture would test the seed, not the FAB. (The `standard_library.rs` harness is native-only and runs `parse → analyze_local → lower` with no live system, so "render the FAB against a seeded branch" isn't expressible there anyway.)

What CAN drift, and what these tests pin:

1. **Claim ↔ handler attribute agreement.** The FAB hand-builds claims; handlers index on attribute URIs (`dialog-reactor/src/command.rs:52-83`). If the two disagree, the command decodes as nothing, the handler never runs, and the UI still looks successful — the exact failure this design exists to prevent. `trigger_attributes()` makes it exactly testable.
2. **Query ↔ written-attribute agreement.** The FAB reads a raw attribute; the worker writes facts under the schema's domain type. Diverge and the chip silently blanks.

Full end-to-end against a real old spot stays a browser check (Task 11, Step 5) — honest about what these tests do and don't cover.

**Files:**
- Create: `rust/tonk-worker/tests/fab_drift.rs`
- Modify: `rust/tonk-worker/Cargo.toml` (add `tonk-fab` to `[dev-dependencies]`)

- [ ] **Step 1: Add the dev-dependency**

`tonk-fab`'s `logic` module is native (`pub mod logic;` is ungated), so the
worker's native test can call it. Add to `rust/tonk-worker/Cargo.toml`:

```toml
[dev-dependencies]
tonk-fab = { path = "../tonk-fab" }
```

- [ ] **Step 2: Write the failing test**

Create `rust/tonk-worker/tests/fab_drift.rs`:

```rust
//! The FAB must keep working against a space branch seeded by ANY past
//! `core.yaml`.
//!
//! `core.yaml` is seeded once at repo creation and never re-seeded, so every
//! existing space's descriptors are frozen at the version that created it.
//! The FAB survives that by consulting nothing seeded: it reads raw attribute
//! URIs and inlines its own command descriptors. That is why there is no old
//! -library fixture here — there is nothing seeded for it to be checked
//! against.
//!
//! The load-bearing invariant is that a hand-built claim carries exactly the
//! attributes its handler indexes on. If they drift apart the command decodes
//! as nothing, the handler never runs, and the UI still looks successful —
//! the precise failure this design exists to prevent.
//!
//! Native-only, mirroring `standard_library.rs`: no filesystem on wasm, and
//! this needs no running system.

#![cfg(not(target_arch = "wasm32"))]

use tonk_fab::logic;

#[dialog_common::test]
fn it_builds_rename_claims_carrying_every_attribute_the_handler_triggers_on() {
    use dialog_reactor::Decode as _;

    let triggers = tonk_schema::command::RenameRepository::trigger_attributes();
    assert!(!triggers.is_empty(), "the command declares trigger attributes");

    let claim = logic::rename_repo_claim_json("did:key:z6Mk", "Renamed").to_string();
    for attribute in &triggers {
        assert!(
            claim.contains(attribute.as_str()),
            "hand-built rename claim must carry trigger attribute {attribute}"
        );
    }
}

#[dialog_common::test]
fn it_builds_invite_claims_carrying_every_attribute_the_handler_triggers_on() {
    use dialog_reactor::Decode as _;

    let triggers = tonk_schema::command::Invite::trigger_attributes();
    let claim = logic::invite_claim_json("did:key:z6Mk", 1.0).to_string();
    for attribute in &triggers {
        assert!(
            claim.contains(attribute.as_str()),
            "hand-built invite claim must carry trigger attribute {attribute}"
        );
    }
}

#[dialog_common::test]
fn it_reads_the_repo_name_attribute_the_schema_writes() {
    // The FAB reads a raw attribute; the worker writes facts under the
    // schema's domain type. If the two diverge the chip silently blanks.
    let body = logic::repo_name_query_body("did:key:z6Mk").expect("builds");
    assert!(body.contains("xyz.tonk.repo/name"));
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test -p tonk-worker --test fab_drift`
Expected: FAIL — `rename_repo_claim_json` / `invite_claim_json` not found until Tasks 4 and 8 land.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p tonk-worker --test fab_drift`
Expected: PASS, 3 tests.

- [ ] **Step 5: Commit**

```bash
git add rust/tonk-worker/tests rust/tonk-worker/Cargo.toml
git commit -m "test(tonk-worker): pin FAB claims against handler trigger attributes"
```

Note: `standard_library.rs` reaches the analyzer as
`tonk_analyzer::analyzer::analyze_local` (not `tonk_analyzer::analyze_local`).
Copy any lowering invocation from there verbatim rather than guessing.

---

### Task 11: Full verification

- [ ] **Step 1: Lint gate**

Run: `cargo clippy --workspace --all-targets --all-features && cargo fmt --check`
Expected: clean. `--all-features` compiles integration tests — a per-crate clippy passing does not mean this does.

- [ ] **Step 2: Full test suite**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 3: wasm tests**

Needs Chrome at the default `/Applications` path plus a major-matched chromedriver.

Run: `cargo nextest run -j1 --target wasm32-unknown-unknown`
Expected: PASS.

- [ ] **Step 4: Fix the stale comment**

`repository.rs:968` calls the repo key the DID's "suffix". `repo_key()` returns `self.as_str()` — the full DID (`prelude.rs:80-93`). Correct the comment.

- [ ] **Step 5: Manual verification against a real old space**

Open a spot created *before* this branch — ideally one whose branch predates #572. Confirm every FAB zone renders and works: name (read + rename), share (mint + copy), roster, switcher, sync disc, pause, drag/dock, wizard. This is the whole point of the change; no automated test covers the real browser path end to end.

- [ ] **Step 6: Commit and open the PR**

```bash
git add -A
git commit -m "fix(tonk-fab): correct the repo-key suffix comment"
git push -u origin feat/fab-web-component
gh pr create --base staging --title "feat(tonk-fab): extract the FAB into a web component" --body "..."
```

**Base the PR on `staging`, not `main`.**
