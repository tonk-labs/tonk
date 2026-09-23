# Plan 005: Bring the hub and account settings up to the latest wireframes

Status: IN PROGRESS. Priority: P1. Effort: L overall; first layout slice M. Risk: medium for layout, high for account integration and copying spaces. Planned against `d4003e8d6` on 2026-09-22. No dependency on the earlier CLI plans.

This is an implementation handoff, not an implementation. Work in the existing `feat/new-hub` worktree, preserve unrelated changes, and deliver the increments below independently. If commits are requested, commit each proven increment separately using the repository's conventional style, for example `feat(hub): render responsive space cards`. Do not publish or deploy as part of this plan.

## Design source and evidence

The reference is `/Users/jackdouglas/tonk/gooey/fabb/hub.html`, including its final CSS overrides and `hub-settings.css`, `hub-walkthrough.js`, `account-demo.js`, and `fabb.js` alongside it. Read the composed page, not just its first stylesheet. The latest card treatment is explicitly dated September 21: weighted frames around blurred previews. The earlier directory/index/album studies and the earlier paper-offset cards are not the target.

Implementation reference snapshot (2026-09-22): gooey revision `f74da0ee5addabaff3df6039f9630ede2c2a8ed9`; SHA-256 `hub.html` `bf9f9f9b0ab6a247ca244d5ced84531da99a55e8c369d684236ea5e78246819f`, `hub-settings.css` `51500a5ba32bafd4e7da4257eeb1dbbb8da66256d5287d3a44648a98d1d0bbb4`, `hub-walkthrough.js` `3ff2ab9e277c3027e4f56ff340df5f9b015445344c0c0151917fb6ed7ccceba9`, `account-demo.js` `88ba3e39a4f7ad7eba4722a0b06b35f31c3af4d39e1858936f361ee293d8c132`, and `fabb.js` `ebdd80f770e156cff48e182db29ddae52038064298a9ef650d7f16435f6ed4fb`.

During planning, the reference was served locally and inspected in isolated Chrome: desktop hub, mobile hub at 390 × 844, and account settings accessibility snapshots on desktop and mobile. Source inspection covered the walkthrough scenarios and their simulated actions. No running production checkout was built or browser-tested; current-product comparisons below are from source. The initial sandbox blocked local serving and browser startup; both worked with host access. The reference console reported a form-field id/name issue, not a JavaScript exception. Its browser actions are demonstrations, not evidence that corresponding product operations exist.

Before implementation, run:

```sh
git status --short
git diff --stat d4003e8d6..HEAD -- rust/tonk-core/assets/library rust/tonk-ui rust/tonk-worker rust/tonk-schema rust/tonk-workspace
```

Recheck changed code against the evidence below. Refresh this plan if the reference changed. Record the reference revision or file hashes in this file when implementation starts, since it lives outside this repository.

## Outcome and architectural decision

Replace the centered launcher with a full-width collection page, framed preview cards, a desktop header, and mobile Spaces / New space / Account navigation. Bring `/settings` to the separate-panel account design and retain working account, local-space, invitation, and device-link behavior.

Keep the existing architecture. `rust/tonk-ui/src/bin/ui.rs:33` installs a thin host and one `<tonk-site>`; the profile's route table and sealed guests render the pages. Despite older README prose describing Leptos and top-document settings, the current executable source does not have a Leptos hub to redesign. Do not introduce a second SPA or duplicate state in `tonk-ui`.

The main implementation belongs in `rust/tonk-core/assets/library/profile.yaml`: `view!` markup/styles, existing concepts and commands, and small `element!` behaviors. Restrict Rust shell changes to the real top-document ceremony, focus, and anchoring needs. Do not revive the removed `ui_hub_account.rs` implementation from older branches.

## Current state and concrete gaps

| Area | Current evidence | Target and implementation implication |
| --- | --- | --- |
| Page structure | `profile.yaml:369` limits `.hubcol` to 576px; `:564` renders the account/spaces bar and vertical `.stack` | Header wordmark, desktop New space/account actions, collection panel, responsive card grid. High confidence, M effort. |
| Space rows | `profile.yaml:16–106` declares subject, name, presence, owner, provider, home address and founding time; `:637` renders links from directory facts | Preserve stable subject identity and remote/seeding states. Description, meaningful activity time and real preview data are additional contracts, not existing fields to interpolate. High confidence, M/L effort. |
| Card style | Reference `hub.html:287–349`, `:537–588` implements seeded frame weights and generated preview blocks | Desktop preview above caption; mobile 64px preview beside caption. Stable decorative frames are feasible now. Generated blocks are explicitly stand-ins for content. High confidence, M effort. |
| Navigation | `profile.yaml:1714` defines `hub-bar`; `:1724` seats ceremonies using `.hubbar` | Real `/` and `/settings` routes, responsive navigation, an explicit ceremony anchor independent of the retired bar geometry. High confidence, M effort. |
| Create/actions | `profile.yaml:122–154` creates an Untitled space and opens it; `:637–677` offers copy-link and authority-aware removal | Reference has a name/optional-description create dialog and More → rename/duplicate/delete. Rename has a worker provider; duplication needs a real content-copy operation. Preserve existing copy-link access and removal distinctions. High confidence on visual gap, medium on copy implementation, L effort. |
| Settings | `profile.yaml:1621–1697`, `:1969–2033` already renders account facts, passkeys, sign-out, deletion and agent access | Separate Your details, Passkeys, Switch account, Sign out, and Delete account surfaces. `hub-settings.css` supersedes boxed styling with borderless tonal panels and 48px controls. High confidence, M effort. |
| Discover | Reference `:525`, `:590–595`, `:626`, `:630` uses fictional records and copies JavaScript objects | Needs curated content and safe independent copies. No catalog/copy contract was established in this review. High confidence that the demo is simulated; L effort to implement and verify. |
| Account state | Reference `:510–515` uses demo state/network connectivity; walkthrough adds unverified-email notice | Subscribe to actual account/profile sync and registration facts. Online connectivity is not proof of synchronization. High confidence, M effort. |

Relevant current code shapes to preserve:

```html
<!-- profile.yaml:637 onward: the subject is stable; actions are outside the link. -->
<div class="srow-wrap" data-presence={presence} data-id={this}>
  <a class="srow blk" href="/space/{subject}"><span class="n">{name}</span></a>
  <space-remove data-space-subject={subject} data-space-name={name}
    data-space-provider={provider} data-space-founded={founded-at}
    data-space-owner={owner}>
```

```html
<!-- profile.yaml:675: creation is a command, not local JavaScript state. -->
<form class="snew-form" on:space-create=space/create>
  <input type="hidden" name="name" value="Untitled">
  <input type="hidden" name="open" value="true">
```

`hub-bar.place` passes `window.tonk.register` a serialized `{ reason, space: '', anchor }`; `seat` returns null for a missing/zero rectangle. Retain that protocol and its reseat/open distinction while changing the anchor. `rust/tonk-ui/src/register_dialog.rs` owns the top-page registration UI; `custody_relay.rs:177–204` validates and applies anchors. WebAuthn must continue to start through the existing top-document user-gesture path.

Follow the existing `event!` → typed command → worker provider pattern. Event extraction uses DOM properties such as `.currentTarget.elements.name.value`; element method names are hyphenated in YAML and camel-cased at runtime. Use fact subscriptions through `tonk-display`, not polling or a second localStorage account model. Keep native input elements for editing and `tonk-dialog` for modal focus management.

## Scope

Primary implementation files:

- `rust/tonk-core/assets/library/profile.yaml`: hub, settings, local UI behaviors and command declarations.
- `rust/tonk-ui/src/register_dialog.rs`, `custody_relay.rs`, `ceremony.rs`, `account.css`: only ceremony presentation/placement that the new shell requires.
- `rust/tonk-ui/src/account_flow.rs`, `helpers.rs`, and `rust/tonk-worker/tests/standard_library.rs`: focused regression coverage and intentional selector updates.
- `rust/tonk-ui/README.md`: correct the obsolete rendering description when documenting this change.

Conditional scope for the capability increments: `rust/tonk-worker/src/router/repository.rs`, `command.rs`, `transfer.rs`, `account_state.rs`, `profiles.rs`, `profile_name.rs`; `rust/tonk-schema/src/command.rs`; relevant directory/schema definitions discovered through their imports; `rust/tonk-workspace/src/ui_sync_status.rs` if the existing subscribed indicator needs account-context support. New catalog/preview/copy files must be named in this plan after their spikes establish the contract. Do not expand into these files for the initial layout increment.

Read `docs/storybook/AGENTS.md` before any Storybook edits. If its impact workflow applies, add the affected stories and regenerate through its documented commands rather than hand-editing generated data.

Out of scope: dependency upgrades, lockfile changes, a new rendering framework, service-worker lifecycle changes, authentication/UCAN redesign, resetting local data, changes to space canvas/FABB dragging, registry publication, deployment, and porting prototype controls or simulated accounts into production. Broadly rewriting `core.yaml` is not required for a hub refresh.

## Implementation order

### 1. Prove the smallest responsive hub slice

First establish the existing lowering and hub-browser baseline using the commands below. Add a real-data card layout in `profile.yaml` using the existing `tonk:space` directory, subject link, presence attributes, create command, and removal controller. Start with a neutral decorative preview placeholder and the known name. Do not fetch or mount every space to populate the hub.

Replace the narrow column with the reference's centered, roughly 1250px-max collection area, desktop header and responsive grid. At widths up to 600px, use the horizontal thumbnail/caption cards and fixed three-action navigation with safe-area padding. Make the wordmark an actual home link. Use route links for Spaces and Account, preserving browser back/forward and direct `/settings` navigation. Keep New space reachable in the desktop header, the collection's trailing card, and the mobile center action, all dispatching the same operation.

Keep the existing creation behavior in this first independently working increment. Do not expose inactive Discover or Duplicate actions yet; they arrive in increments 4–5. This intermediate state is explicitly not full wireframe parity.

Extract the final frame grammar, not the entire layered prototype stylesheet: 1/3/7/15px desktop rule weights, smaller mobile weights, top hairline, right post, fractional bottom sill and left foot. Seed from stable subject identity, never DOM row index or random-on-render. The reference's neighbor adjustment can alter the final weight when neighboring rows change; test deterministic output for the same ordered list rather than claiming absolute frame immutability. Menu/dialog decoration should be deterministic too. Keep focus indication distinct from decoration.

Verification: standard-library gate plus `it_dresses_the_hub_from_the_view_that_declares_its_style`; add a focused `it_renders_the_responsive_hub_collection` browser test covering navigation and overflow at desktop/mobile widths. At least one linked account and one local-only profile must open a real existing space and create a new one from the new layout. Expected: working navigation, one create per activation, no nested interactive controls or clipped last card.

### 2. Move settings and ceremony anchors together

Recompose the existing fact-driven settings views into the final tonal panels from `hub-settings.css`: two columns on desktop, one on mobile, full-width deletion section. Reuse `tonk:account/passkey` rows and the real roster; switch by durable handle, never the display label. Preserve add-account access, agent access management, `/settings/link` approval/local-link panes, and existing sign-out/deletion confirmation controllers even though the reference omits some of them.

Change display-name editing from implicit input change to the reference's explicit Save changes form, using the existing rename command with a submit event that reads the form control. Validate trimmed non-empty input; keep pending/error feedback associated with the form. Prove a rerender cannot submit a second rename or erase an active unsaved edit. Do not manufacture passkey nicknames: render the existing created-on/created-at facts; adding a nickname field is separate capability work if wanted.

Replace `.hubbar` as an implicit ceremony anchor with a deliberate visible seat shared by desktop/mobile layouts. Retain `hub-bar` as the controller initially if that reduces migration risk; its tag name need not dictate visible layout. Adapt the top-document positioning only as needed for viewport clamping, scroll/resize, keyboard space and detached opener recovery. Native dialogs must restore focus after cancel and after profile rerenders. At mobile sizes, a bottom-nav account button's bottom edge must not place a dialog below the viewport.

Verification: standard-library gate and focused browser tests for signup, cancel/retry, second account switching, sign-out/re-entry, account deletion, and terminal-link refusal listed below. Add `it_keeps_hub_ceremonies_visible_on_mobile` with viewport/anchor bounds and focus restoration assertions. Expected: settings and retained link panes remain usable, user gestures still reach WebAuthn, and unrelated retained local spaces survive switching/sign-out.

### 3. Add honest account and collection states

Add the 14px account sync mark and accessible status text to both account entry points. Start by proving the profile/account context exposes the needed live sync facts. `ui_sync_status.rs` is an existing subscription example, but its documented context is a space; do not assume mounting it against the profile proves account synchronization. Keep pending, disconnected, paused and failed states distinct where the worker provides them. Never show “account synced” solely because `navigator.onLine` is true.

Add the unverified-email reminder from the walkthrough, backed by the existing account registration/activation subscription and resend flow. Place it above mobile navigation without covering focused content. Pending activation, sign-out, profile switch and activation from another tab must update/remove it without a reload.

Introduce a real settled-empty state only after directory readiness is known. The existing comment at `profile.yaml:628` deliberately avoids confusing an empty directory with one still arriving. Prove a readiness signal before showing “No spaces yet”; use a loading state while unresolved and a recoverable error state on failure. Retain directory-only remote cards and the downloading route; viewing the collection must not trigger replication of all cards.

Verification: focused browser tests `it_reads_the_account_state_the_bar_subscribes_to`, `it_waits_for_the_email_when_a_second_device_signs_in`, `it_returns_a_new_browser_login_to_the_synced_hub`, plus new `it_distinguishes_loading_empty_and_failed_hub_collections`. Expected: delayed account data never flashes a false empty/synced state; offline local spaces remain accessible. If no reliable account sync/readiness projection exists, add and test that projection before enabling these labels.

### 4. Complete create, rename, removal and card metadata

Implement the reference's create dialog: required trimmed name, optional description, cancel, busy state, inline error, and navigate after successful creation. Preserve the ability to create local-only spaces without an account. Keep the old Untitled command behavior valid for FABB/CLI and older seeded libraries; update the hub form rather than globally requiring new fields.

First spike one named local space with a persisted description. Establish its canonical fact and the directory mirror before broadening. The existing space concept has no description/activity/preview fields. Add optional fields only with corresponding writers and migration-safe readers. If descriptions cannot be written atomically with creation, specify recovery so a partial failure cannot create a second space on retry. Prove reload and a second device can read the directory description without eagerly opening every repository.

Add More → Rename using the existing `RenameRepository` provider in `repository.rs:2351`, which validates the target and updates the directory mirror. Keep native inputs and stable subject targeting. Preserve copy-link access in More as an existing product capability. Route deletion/leave through `space-remove` with provider/owner/founded metadata; do not adopt the prototype's universal “delete” operation or erase another member's hosted space. Menus must work with keyboard, touch, Escape, click outside, viewport edges and focus return.

For preview and activity metadata, use staged contracts: ship a clearly decorative fallback first; omit unknown description/time rather than fabricate it. Do not relabel `founded-at` as “last updated.” A real preview needs an explicit production source (for example a user-selected cover or a bounded locally cached thumbnail), a revision/invalidation policy, account isolation, and remote/offline fallback. Do not create a background screenshot service or eagerly instantiate live space iframes as an incidental styling change. Record the chosen source and a one-space working spike before implementing it across the collection.

Verification: worker tests for new field persistence/backward compatibility plus `it_creates_a_local_only_space_from_the_hub_wizard`, `it_removes_a_space_without_letting_focus_escape_the_sealed_guest`, and new `it_renames_a_space_from_the_hub`. Expected: one repository per create, source-of-truth rename reflected in directory, cancellation leaves data unchanged, and local/owned/joined removal retains its existing scope. Full preview parity remains tracked as incomplete until a real source is chosen and verified.

### 5. Prove copying, then connect Discover

Treat duplication as a data feature. The reference only clones an object in localStorage. The existing CSV transfer paths in `rust/tonk-worker/src/router/transfer.rs` are useful leads, not a proven complete-space copy implementation: branch content, schema, blobs, identity and governance must all be accounted for.

Run a bounded spike that copies one local space containing a custom view and a blob into a new repository identity. Prove edits are independent and membership, invitations, provider grants and source ownership are not copied. Check failure midway, repeated activation, remote-only source, offline source and naming collisions. Keep the original untouched and expose a new card only once the copy is usable. Turn the proven operation into one typed command with progress/error feedback; reuse it for Duplicate and Discover's Make a copy.

For Discover, the recommended first version is a small versioned bundled catalog of explicitly distributable templates, not a new public catalog service. The content owner still needs to choose the actual entries and confirm their distribution/copy rights; no real entries were supplied in this task. Record the catalog format, repository asset path, provenance, schema/blob dependencies and offline behavior in this plan before implementation. Use fixture entries in tests, not fictional production community data.

Then enable Your spaces / Discover, reuse the card view, add preview with author/description, and Make a copy → a new owned space. A settled zero-space collection shows create plus inline Discover as the reference does; the no-results/loading/error states must not depend on sample record counts. Keep collection selection stable across harmless fact rerenders; never leak a previous account's cards after switching.

Verification: new worker copy tests and `it_duplicates_a_space_without_copying_access`, then browser `it_copies_a_discover_template_into_an_independent_space`. Expected: source and destination differ in identity, original content remains unchanged, source grants are absent, blobs/views work, retries do not add duplicate completed copies. Catalog unavailable/offline behavior is tested. Do not mark Discover complete with a dead tab or fixture-only flow.

### 6. Integrate, upgrade existing profiles, and capture parity

Run the complete focused browser group after the last product change. Test an existing seeded profile upgrading to the new library, not just a clean browser. `repository.rs:5336` and `:5812` already reconcile profile libraries; extend the existing reconciliation regression fixture rather than clearing storage or adding another migration system. Preserve authored profile facts and the person's spaces through the upgrade.

Capture hub and settings at 0/1/5/25 spaces, 390 × 844 and desktop, light/dark, and at least one intermediate width. Also cover long names, missing metadata, remote/seeding cards, menus near edges, reduced motion, keyboard-only operation and mobile safe areas. Validate Safari editing/focus/ceremony placement separately from Chrome; report unavailable Safari/device evidence explicitly.

Reconcile walkthroughs as presentation coverage, not new authority semantics: first visit, returning account, signed-out device, unverified email, passkey cancel/retry, invite with zero/one/multiple accounts, and unavailable/revoked invite. Keep the current Welcome-once behavior (`ui.rs:186`, existing test below) until a separate product decision replaces it. The reference landing headline `TKTKTKTKTK`, arbitrary sync-server form, demo invitation parsing and “agent connected” fixtures are not production requirements. Existing access and agent-receipt truth must survive the visual changes.

Verification: full standard-library test target, profile-library reconciliation tests, web build, selected browser suite and story impact checks. Expected: fresh and upgraded profiles render the same new UI without data loss; all explicitly retained flows pass. Record commit, commands, results and screenshot locations in this file and update the index.

## Verification commands and test patterns

These commands are derived from `flake.nix:246`, `:419–443`, the crate manifests and the existing tests. They were inspected, not executed during this planning task. Run from repository root in the configured Nix environment; distinguish infrastructure failures from product failures.

```sh
# Narrow schema/markup gate; run at each profile.yaml checkpoint.
nix develop --accept-flake-config .#ci --command cargo test -p tonk-worker --test standard_library

# Existing profile upgrade preservation tests; integration checkpoint.
nix develop --accept-flake-config .#ci --command cargo test -p tonk-worker --lib profile_library_

# Build the actual served app at meaningful browser/integration checkpoints.
nix build .#tonk-ui

# Focused real-browser test, through the repository's serialized E2E harness.
nix develop --accept-flake-config .#ci --command test:e2e -E 'test(it_dresses_the_hub_from_the_view_that_declares_its_style)'

# Rust and patch hygiene after the final implementation change.
cargo fmt --all -- --check
git diff --check
node --test rust/tonk-ui/tests/style-boundaries.test.mjs
```

Use `test:e2e -E 'test(NAME)'` with each named test above; combine filters with `or` for integration runs. Confirm the selected test count is nonzero. Newly proposed test names do not exist yet and must be implemented before using them as gates. Model browser tests on `account_flow.rs:2238`: enter the routed guest using existing helpers and assert computed styles/behavior there, not on the unrelated top document. Test semantic behavior rather than perpetuating the retired `.hubbar` geometry. Update stale static layout assertions intentionally, while retaining lowering, authority, command binding and accessibility checks.

Additional existing browser regression names for the integration group:

- `it_opens_the_welcome_space_once_then_the_hub`
- `it_finishes_signup_in_the_original_tab`
- `it_retries_the_committed_address_after_a_failed_passkey_ceremony`
- `it_adds_a_second_account_and_switches_between_disjoint_space_lists`
- `it_signs_back_into_the_same_account_after_signing_out`
- `it_signs_into_another_account_without_rebinding_retained_local_spaces`
- `it_deletes_the_account_and_releases_its_email_and_profile`
- `it_replaces_agent_link_progress_with_the_account_handoff_refusal`

## Completion and stop conditions

### Implementation result (2026-09-22)

- Increments 1–2 are implemented: the Hub uses a responsive real-data card grid, stable subject-seeded weighted frames, a fixed mobile navigation bar, a home-linked mark, and shared tonal settings panels with explicit display-name submission and native inputs.
- Increment 3 is partial. The account ceremony now anchors to the visible desktop account control or mobile account destination, and an unverified account gets a fact-backed resend reminder. A truthful account-sync indicator and settled zero-space state remain open because the profile branch does not expose reliable readiness facts for either state; no loading or success state was fabricated.
- Increment 4 is implemented for the selected contracts: the named create dialog validates a required trimmed name and optional description; creation writes the description into the repository seed and mirrors it into the account directory with the initialized status; cards render a decorative local fallback and omit absent metadata; More provides copy link, native rename, and the existing authority-aware remove/leave flow. Real preview and activity sources remain deliberately unimplemented.
- Increment 5 is paused at its stated dependency boundary. No production catalog contents, source contract, or independent copy operation were supplied, so Discover and Duplicate are not represented with demo records or simulated actions.
- Fresh evidence after the final product change: 50 standard-library tests and 18 profile-library reconciliation tests pass; the real-Chrome Hub group selects and passes 4/4 tests (`it_renders_the_responsive_hub_collection`, create with persisted description, rename, and authority-aware removal with focus trapping/restoration). The served UI artifact was rebuilt by the E2E harness. The style-boundary test, `cargo fmt --all -- --check`, and `git diff --check` pass. Earlier focused gates also passed 82 `tonk-display` tests and 5 description/backward-compatibility tests.
- The browser investigation caught two concrete integration faults before completion: fixed create-dialog buttons used literal `html:form` and therefore had no form owner, and the open ancestor menu prevented a nested removal dialog's native Escape action. Both boundaries now have static contracts and final Chrome coverage.
- Safari/device, light/dark screenshot parity, the 0/1/5/25-space visual matrix, and the broader retained account/invitation browser group remain unrun. The temporary configured-cache DNS/timeout failure was retried unchanged and is not a product result.

- [ ] Increments 1–3 deliver real responsive hub/settings navigation with retained account and local-space behavior.
- [x] Increment 4 persists create metadata, supports rename and authority-aware removal, and records the implemented preview contract or explicitly outstanding gap.
- [ ] Increment 5 proves independent copying and uses a chosen production catalog; no simulated production records/actions.
- [x] All selected implementation gates exit 0 with nonzero matching test counts after the final change; failures and unrun browser/device cases are recorded.
- [x] Existing-profile upgrade preserves authored facts and spaces; no cache/storage resets are needed.
- [ ] Visual artifacts cover the stated viewport/theme/state matrix; no prototype toolbar, reset/shuffle controls or placeholder landing copy ships.
- [x] `git diff --check` passes and changed paths stay within the agreed scope; index status reflects actual completion.

Pause the dependent increment and report evidence if real previews need a new hosted service, catalog content is unavailable, copy requires undocumented authority changes, or a data model assumption fails. Continue independent layout/settings work. Do not silently substitute demo behavior or call the whole plan done because the first layout slice ships. If a baseline fails before edits, show the error and current hypothesis before changing product code.

Maintenance: keep hub/settings shared tokens in one schema-owned style, keep demo-only controls outside the product, and review selector changes alongside ceremony/E2E callers. Future preview metadata should remain optional so old and remote directory rows render. Catalog and duplication changes should share one tested copy operation. This review did not audit unrelated crates, production services, deployment readiness, or all invitation internals.
