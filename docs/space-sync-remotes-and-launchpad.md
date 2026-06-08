# Space sync remotes, the empty-space launchpad, and the persistent topbar

A session writeup. The goal we started with: **let users choose a sync remote when creating a space, and attach one after the fact.** Getting there surfaced a chain of architectural realities and bugs in the declarative space-creation flow. This documents what we built, every bug we hit, and why the final design looks the way it does.

The work spans `tonk-schema`, `tonk-worker`, `tonk-core` (notation), `tonk-workspace`, and `tonk-ui`, stacked on commit `53d3b28a4`.

---

## 1. The starting point: space creation is a declarative effect

The worker side was already done in an earlier commit (`4495`): `PUT /api/repository/{name}` wires remotes at creation, and `POST /api/repository/{repo}/remote` (`attach_remote` → `ensure_remote_config`) attaches one later. So the remaining work looked like front-end plumbing.

It wasn't, because a rebase mid-session had replaced the old Leptos `create_space()` dialog with a **declarative effects model** (Hub PR #489):

- `profile.yaml`'s Hub renders `<tonk-display model="space">`; its "New space" form is `<form onsubmit=space/create>`.
- Submitting asserts a **transient `CreateSpace` concept** (`tonk-schema/src/command.rs`) whose fields are `dom.event.*` read-paths (`tonk-schema/src/domain.rs::command`). The notation event layer (`tonk-display/src/events/`) reads the form fields and POSTs a transient claim to `/api/profile/branch/meta/transact`.
- After commit, the worker's `dispatch()` matches the transient against registered command **handlers** and runs them.

So both "create with a remote" and "attach later" had to become **declarative commands**, not Leptos dialogs.

### Key facts about command matching (these drive everything below)

- A command **matches by decoding its concept from the committed transient's facts**. `match_transients` groups transient artifacts by entity, looks up candidate handlers by the *attributes* the entity touches, and keeps those whose concept decodes.
- **Every declared concept field is required.** `Option<T>` is unsupported — the `Concept` derive does `<Field as Attribute>::Type`, and `Option<RemoteUrl>` isn't an `Attribute`.
- The **profile meta branch is seeded once per profile and not re-seeded across versions** (`bootstrap_profile_meta` → `seed_profile_library`, fetching the served `/library/profile.yaml`). So a profile's `space/create` *descriptor* is frozen at the version that created the profile.
- A `Provider<C>` only receives the **decoded command**, never the raw facts. `CommandHandler::run` receives the `EntityFacts`.

---

## 2. What shipped (final design)

### Create-time and enable-later are one handler

There is **no `EnableSync` concept**. A single custom `CreateSpaceHandler` (`tonk-worker/src/router/repository.rs`, registered via `registry.register(Box::new(...))`, not `.command::<>()`) serves **both** forms:

- `CreateSpace` stays `{ this, name }` — name-only, so it keeps decoding against an older frozen descriptor.
- The handler is keyed on the shared `…elements.name/value` attribute, so it matches both the Hub "New space" transient and the topbar "Enable sync" transient.
- It reads the optional remote URL **straight from the facts** via `remote_from_facts` (see §3.5 for why), then:
  1. **Creates-or-reuses** the repo (`create_space_inner` no-ops an existing one), so the space always appears regardless of the remote.
  2. **Best-effort attaches** the remote via `enable_sync_inner` → `ensure_remote_config` (idempotent). A remote/auth failure leaves a working local space, retryable from the topbar.

So new-space (new repo, maybe a remote) and enable-sync (existing repo, a remote) are the same code path.

### The UI surface

- **Hub "New space" dialog** (`profile.yaml`): a `Sync server (optional)` `<wa-input name="remote">` and a `<tonk-default-remote>` button that fills it with this server (`origin + /ucan/`). Blank → local-only.
- **Topbar** (`core.yaml` workspace directory view): brand, a `‹ all repos` crumb, the space title (a new `<tonk-repo-name>` element), the live `<tonk-sync-state>` chip, `<tonk-sync-toggle>`, `<tonk-share>`. On a no-upstream space `<tonk-sync-state>` shows an **Enable sync** trigger that opens a notation `<wa-dialog>` (same remote-URL field + default-service button), bound to `onsubmit=space/enable-sync`.
- **Empty-space launchpad**: `<tonk-fallback>` chrome ("This space is empty"), revealed when the directory `<tonk-display>` is `data-state="empty"`.

New custom elements added to `tonk-workspace`: `<tonk-default-remote>` (fills a form input with `origin + /ucan/`) and `<tonk-repo-name>` (renders the repo name from the `<tonk-repository>` route ancestor).

---

## 3. The bugs, in the order we hit them

### 3.1 Required field broke all space creation (frozen descriptor)

**First attempt:** add a required `remote` field to `CreateSpace`.

**Symptom:** creating a space with a sync server did nothing — no card, can't navigate. The worker logged `transact profile branch=meta` (the transient committed) but **no `command CreateSpace`** afterward — `dispatch` ran no handler.

**Root cause:** the profile's `space/create` descriptor is frozen (profiles aren't re-seeded across versions). A required `remote` field meant the worker's `CreateSpace{name, remote}` could only match a transient that carried a `remote` fact; against the older name-only descriptor it never matched. `dispatch` committed the transient and silently ran nothing (the dialog still closes via `data-dialog`, so it *looks* successful). This would break creation for **every existing user**, not just locally.

**Fix:** `CreateSpace` stays name-only. Regression guard: `it_decodes_create_space_from_name_only_facts`.

### 3.2 Clicking a new space showed a blank page

**Symptom:** once creation worked (local-only), the new space listed in the Hub but opened to a blank page; `home` rendered fine.

**Root cause:** `home` gets the showcase demo (which seeds a `workspace` instance); a `CreateSpace`'d space gets only the scaffold (`core.yaml`), so it has **zero workspace instances** — genuinely empty. The bare `/space/{name}` route renders the `workspace` *directory* view, which goes to `data-state="empty"`. A `<tonk-fallback>` launchpad element existed and was registered (commit `dc98568a`) but had **never been placed in any view template**, so empty rendered nothing.

**Fix:** wire `<tonk-fallback>` into the `view/directory!: id:tonk-workspace/directory` template as chrome (it carries no `{this}`, so the renderer keeps it mounted regardless of instance count).

### 3.3 The topbar only showed on `home`

**Symptom:** the topbar (brand, sync controls, share) appeared on `home` but not on fresh spaces.

**Root cause:** the topbar was part of the per-instance workspace *entity* view, which only renders when a `workspace` instance exists. `home` has one; a fresh space doesn't.

**Fix:** hoist the topbar (and the enable-sync dialog) up to the always-rendered workspace *directory* view. Its CSS moved to the global `tonk-ui/styles.css` (the directory view always renders; the entity view's `<style>` doesn't). The title can't use the per-instance `{name}` at the directory level — and the workspace `{name}` is a display label, not the repo name anyway — so a new `<tonk-repo-name>` element reads the real repo name from the `<tonk-repository>` route ancestor. Bonus: this makes the **Enable sync** trigger reachable on empty spaces.

### 3.4 The launchpad title was invisible in dark mode

**Root cause:** the global `h1..h6` rule in `styles.css` bakes in a "headline-cover" (`background: var(--wa-color-text-normal); color: var(--wa-color-neutral-fill-quiet)`) that isn't theme-aware — in dark mode it's light text on a light fill.

**Fix:** opt the launchpad title out (`background: none; display: block`), exactly like `.hub-title` does.

### 3.5 The remote was still `None` — a URL deserializes as an `Entity`

**Symptom:** with everything else working, creating a space with the sync field filled logged `command CreateSpace name=test remote=None`. The captured request payload proved the form posted `parameters: {name: "test", remote: "http://127.0.0.1:8080/ucan/"}` correctly — so the bug was worker-side.

**Root cause:** the worker's untagged `Value` deserialization tries `Entity` first for **any string containing a `:`**. A URL has colons, so `http://127.0.0.1:8080/ucan/` deserializes as `Value::Entity`, not `Value::String`. A `String`-typed concept field (the original `EnableSync.remote: RemoteUrl(String)`, which the create handler decoded to read the remote) **can't decode an `Entity`**, so the decode returned `None`. The name (`test`, no colon) deserializes as `String` and decoded fine — which is exactly why only the remote broke.

Confirmed with a one-line probe:

```rust
serde_json::from_str::<Value>("\"http://127.0.0.1:8080/ucan/\"") // => Entity(http://127.0.0.1:8080/ucan/)
serde_json::from_str::<Value>("\"test\"")                        // => String("test")
```

**Fix:** read the remote **directly from the artifact**, tolerating both representations:

```rust
fn remote_from_facts(facts: &EntityFacts) -> Option<String> {
    facts.iter()
        .find(|a| a.the.to_string() == REMOTE_ATTR)
        .and_then(|a| match &a.is {
            Value::String(url) => Some(url.clone()),     // colon-less / relative path
            Value::Entity(uri) => Some(uri.to_string()), // a URL
            _ => None,
        })
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
}
```

This is what let us **drop the `EnableSync` concept entirely** (it was only there to decode `{name, remote}`) and collapse to one handler. It also fixed the identical bug on the enable-later path, which previously decoded `EnableSync` via `Provider<EnableSync>` and would have failed on any real URL.

Tests: `it_reads_an_entity_remote` (reproduces the exact `Value::Entity` URL case), `it_reads_a_string_remote`, plus the none/blank cases.

---

## 4. Why a custom `CommandHandler` instead of a `Provider`

The optional remote can't be a concept field (§3.1 frozen-descriptor matching, §3.5 URL-as-Entity), so it has to be read from the transient's raw facts. A `Provider<C>` only ever gets the decoded command — never the facts. The dispatch machinery (`tonk-worker/src/reactor/command.rs`) *does* pass `EntityFacts` to `CommandHandler::run`, so a hand-written handler is the hook. `CreateSpaceHandler` implements `CommandHandler` directly: `matches` decodes `CreateSpace` (name) for candidacy, and `run` reads the name from the decode and the remote from `remote_from_facts`.

This required exporting `RunFuture` from `crate::reactor` alongside the already-public `CommandHandler`, `EntityFacts`, `Env`, and `Decode`.

---

## 5. Attribute-keyed dispatch and the benign overlap

Commands match by **attribute**, not predicate identity. `space/create` and `space/enable-sync` both carry the `…elements.name/value` attribute, so both transients match the single `CreateSpaceHandler`. That's intentional now (one handler), but it's worth remembering: if you ever want two genuinely different behaviors for two forms, give them **distinct field names/attributes**, or one handler will catch both. `enable_sync_inner` soft-skips a missing repo (no-op + log, not an error) and `ensure_remote_config` is idempotent, so the overlap is harmless either way.

---

## 6. Notation/form gotchas worth remembering

- **kebab→camel per path segment** (`tonk-display/src/events/path.rs`): `the: dom.event.current-target.elements.remote-url/value` resolves `elements.remoteUrl`. Use single-word field names (`remote`, not `remote-url`) so the input `name` matches the read-path.
- **empty input ≠ unresolved**: `build_transact_body` (`events/extract.rs`) aborts the whole transient if a `dom.event*` field resolves to `undefined`/`null`, but an empty text input coerces to `""`. So a blank field still posts a value; absence of a value means the field wasn't in the descriptor.
- **`data-dialog="open <id>"` is Web Awesome-native** (a global delegated listener), so a button injected by a custom element (e.g. `<tonk-sync-state>`'s Enable-sync trigger) can open a notation `<wa-dialog>`.
- **The workspace `{name}` is a display label, not the repo's local name** (`xyz.tonk.workspace/name`). The repo name lives only on the `<tonk-repository name>` route ancestor; a declarative form can't read it — hence `<tonk-repo-name>` and the `<tonk-sync-state>` stamp-into-hidden-input pattern.
- **`profile.yaml` and `core.yaml` are validated natively** by `tonk-worker/tests/standard_library.rs` (parse→analyze→lower). Run it after any library edit.

---

## 7. Files changed

| Area | File | What |
|------|------|------|
| Schema | `tonk-schema/src/command.rs` | `CreateSpace { this, name }` only; `EnableSync` removed |
| Schema | `tonk-schema/src/domain.rs` | `command::remote::Value` attribute (now only documents the form path) |
| Worker | `tonk-worker/src/router/repository.rs` | `CreateSpaceHandler` (matches name, reads `remote_from_facts`, create-or-reuse + best-effort attach); `enable_sync_inner`; `space_config` |
| Worker | `tonk-worker/src/router/command.rs` | register the one handler |
| Worker | `tonk-worker/src/reactor.rs` / `reactor/command.rs` | export `RunFuture` |
| Notation | `tonk-core/assets/library/core.yaml` | topbar hoisted to the workspace directory view; `<tonk-fallback>` launchpad; `space/enable-sync` command + dialog |
| Notation | `tonk-core/assets/library/profile.yaml` | optional `remote` field + input + `<tonk-default-remote>` in the create form |
| UI | `tonk-ui/styles.css` | topbar CSS (global) + `.workspace__default-remote` |
| Workspace | `tonk-workspace/src/default_remote.rs`, `repo_name.rs`, `sync.rs`, `lib.rs` | new `<tonk-default-remote>` and `<tonk-repo-name>` elements; `<tonk-sync-state>` Enable-sync trigger |

---

## 8. Testing

Verified without a browser (native + compile):

- `it_decodes_create_space_from_name_only_facts` — `CreateSpace` matches a frozen, name-only descriptor.
- `remote_from_facts_tests` — reads a `Value::String` remote, a `Value::Entity` (URL) remote, none, and blank.
- `space_config_tests` — local-only vs `origin`/`origin-main` shape.
- `tonk-worker/tests/standard_library.rs` — `core.yaml`/`profile.yaml`/`demo.yaml` lower cleanly.
- Worker wasm build + `tonk-workspace` wasm build/tests compile; clippy clean on touched crates (native and wasm).

Still browser-only (needs a running app): the create→sync round-trip end to end, the `data-dialog` open, `<wa-input>` value round-trip, and the visual layout of the topbar/launchpad. Local wasm integration tests need Safari/Chrome automation.

---

## 9. Lessons

- **Don't add a required field to a command concept that older profiles already seeded.** Frozen descriptors + all-fields-required = silent no-match. Read optional inputs from the facts in a custom handler instead.
- **Don't model URL/colon-bearing text as a typed `String` concept field.** It round-trips through JSON as `Value::Entity`. Read the artifact directly.
- **Wire new chrome elements into a view.** `<tonk-fallback>` existed and was registered but rendered nothing because no template used it.
- **Space-level chrome belongs at the directory level**, not the per-instance view, so it renders on empty spaces too — and its CSS belongs in the global stylesheet, since the per-instance view's `<style>` doesn't render when empty.
- **Capture the actual request payload early.** The `transact` POST body immediately split "front-end posted it wrong" from "worker decoded it wrong" and pointed straight at the `Value::Entity` deserialization.
