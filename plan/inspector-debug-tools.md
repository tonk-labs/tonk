# Inspector debug tools design and implementation plan

**Goal:** Add an offline-safe, read-only diagnostics panel to each space's
`/inspector` route without changing the profile inspector or performing implicit
sync operations.

**Approach:** `<tonk-inspector>` will render a full-width, collapsed branch
summary before its notebook when its `with` context names a space. Expanding the
native disclosure reveals the complete diagnostics and controls. It will fetch
the existing repository-info response for local metadata, refresh that metadata
after an inspector commit, and contact the configured upstream only when the
user chooses `probe remote`. Repository metadata fetches will enter the portal's
existing `with`/`allow` permission check instead of remaining an ungated
exception.

**Constraints:**

- The panel is read-only: no pull, push, sync, remote attachment, or storage reset.
- A local-only or offline space must still open immediately and keep the notebook usable.
- Full identifiers and hashes remain copyable; visible values may wrap but must not be truncated.
- Profile contexts such as `main@profile:tonk` keep the existing notebook-only UI.
- Reuse the repository's Tonk design tokens and rectangular chrome; introduce no dependency.

## File map

- `rust/tonk-inspector/src/debug.rs`: decode local/sync wire responses and render the panel's observable HTML states.
- `rust/tonk-inspector/src/element.rs`: mount the panel, perform metadata/probe fetches, wire refresh/copy actions, and refresh after commits.
- `rust/tonk-inspector/src/lib.rs`: expose the pure debug-rendering module to native tests.
- `rust/tonk-inspector/tests/debug.rs`: behavioral tests for configured, local-only, sync, and error panel output.
- `rust/tonk-inspector/Cargo.toml`: keep DOM/network dependencies wasm-only where practical and enable clipboard/fetch DOM APIs.
- `rust/tonk-ui/styles.css`: compact responsive branch-panel layout and interaction states.
- `rust/tonk-portal/src/bridge.rs`: classify repository metadata routes under the portal reach check.

### Task 1: Render local repository diagnostics

**Files:**

- Create: `rust/tonk-inspector/src/debug.rs`
- Create: `rust/tonk-inspector/tests/debug.rs`
- Modify: `rust/tonk-inspector/src/lib.rs`

**Interfaces:**

- Consumes: the existing `GET /api/repository/{repo}` JSON shape and resolved repository/branch strings.
- Produces: `debug::render_loading`, `debug::render_repository`, and `debug::render_failure`, returning escaped HTML for the panel body.

- [x] Add `it_renders_full_local_and_remote_diagnostics` with a byte-array tree hash, upstream `origin/main`, UCAN endpoint, repository/profile/operator DIDs, and copy affordances.
- [x] Add `it_names_a_space_without_an_upstream_as_local_only` and a visible local-fetch failure case.
- [x] Run `cargo test -p tonk-inspector --test debug`; expect failure because the debug module does not exist.
- [x] Implement typed, engine-free wire mirrors, full tree-hash formatting, HTML escaping, and deterministic row rendering.
- [x] Run `cargo test -p tonk-inspector --test debug`; expect success.

### Task 2: Mount and operate the read-only panel

**Files:**

- Modify: `rust/tonk-inspector/src/element.rs:TonkInspectorElement::connected_callback`
- Modify: `rust/tonk-inspector/Cargo.toml`
- Modify: `rust/tonk-ui/styles.css`

**Interfaces:**

- Consumes: `resolve_context`, proxied `window.fetch`, `tonk:committed`, and the browser clipboard API.
- Produces: a space-only `.inspector-debug` section with `refresh`, `probe remote`, and delegated `copy` actions.

- [x] Add a browser-facing DOM test that proves a named-space inspector mounts the panel while a profile inspector does not.
- [ ] Run the focused wasm test command; expect the named-space assertion to fail before the panel exists.
- [x] Mount local loading state synchronously, fetch repository metadata asynchronously, and leave notebook startup independent of metadata success.
- [x] Refresh local metadata after a successful inspector transaction and on explicit refresh.
- [x] Probe `/api/repository/{repo}/branch/{branch}/sync/status` only on explicit action, preserving local rows if it fails.
- [x] Copy exact full values through `navigator.clipboard.writeText`, with the pressed button changing to `copied` (or `retry` on failure) and at least a 40px hit area.
- [x] Style a responsive square-cornered panel using existing ink/surface/font variables, explicit transitions, tabular metadata, and `scale: 0.96` press feedback.
- [ ] Run the focused wasm test command and `cargo test -p tonk-inspector`; expect success.

The focused wasm test binary compiles, but its browser execution is blocked by
ChromeDriver 150 versus installed Chrome 152. Native inspector tests pass.

### Task 3: Gate repository metadata through portal reach

**Files:**

- Modify: `rust/tonk-portal/src/bridge.rs:data_plane_location`

**Interfaces:**

- Consumes: host-relative repository metadata and branch data-plane paths.
- Produces: `Location` values for exact `/api/repository/{repo}` and `/api/profile/repository` requests so `handle_host_fetch` applies `with`/`allow`.

- [x] Add classification tests proving exact named/profile metadata paths are gated, permitted branch paths remain unchanged, and unrelated repository subroutes are not misclassified.
- [ ] Run the focused portal test; expect the metadata classification assertions to fail.
- [x] Extend path classification without broadening access or classifying `/remote`, `/invite`, or asset paths.
- [ ] Run the focused portal test; expect success.

The portal's wasm tests compile; runtime execution has the same ChromeDriver
version blocker as the inspector DOM test.

### Task 4: Verify the integrated route

**Files:**

- Modify only files above if verification exposes a defect.

**Interfaces:**

- Consumes: a locally served space at `/space/{id}/inspector`.
- Produces: fresh evidence for rendering, actions, offline/local-only behavior, and console cleanliness.

- [ ] Run `cargo fmt --all -- --check`, `cargo test -p tonk-inspector`, the focused portal tests, and `git diff --check`.
- [ ] Run the repository web build/test command in an isolated target directory if Cargo locking or cross-target artifacts interfere.
- [ ] Open a real named-space inspector in isolated Chrome; verify full metadata, copy, refresh, remote probe or local-only state, narrow viewport wrapping, and no new console errors.
- [x] Inspect the final diff and report any browser or remote-probe scenario that remains unavailable.

The live local-only route loaded metadata and refreshed it with HTTP 200,
reported copy success, preserved the notebook, and wrapped at a 390px mobile
viewport. A configured remote was unavailable, so the probe interaction is
covered by renderer tests rather than a live upstream. Console inspection found
an existing guest custom-element recursive-mutex panic on both the hub and the
inspector; no diagnostics-panel-specific error was observed. The repository web
test command was attempted, but the nested Git-flake build omitted the new
untracked source files; the full live development build succeeded instead. A
follow-up polish pass verified the collapsed full-width summary, conventional
closed/open disclosure arrows, expanded single-column rows, and inline `copied`
button state at desktop and 390px widths.
