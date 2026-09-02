# Visual Storybook implementation plan

## Outcome

Turn `docs/storybook` into the product's navigable, visual, outside-in source
of truth while keeping the existing Markdown audit authoritative. The result
must work for non-engineers in a browser and for engineers and agents from the
repository.

## Invariants

- The journey catalog remains the single source of truth for user-facing
  flows. The visual app derives from it; it does not maintain a second list.
- A screen is one stable user-visible surface or materially different state,
  not every Cartesian product of account, network, and data variants.
- Every journey maps to at least one screen family. Variants and interruptions
  remain in the feature documents and verification checklists.
- Every screenshot names its evidence: running product, production-source
  fixture, or captured CLI output. No mockup is presented as runtime proof.
- Product behavior, local replica state, account authority, customer state,
  access control, and sync remain separate in the visual taxonomy.
- A product-facing change must either update the Storybook or state explicitly
  why its user-visible contract is unchanged.

## Architecture

1. `screens.json` is the machine-readable screen inventory and journey map.
2. `scripts/build.py` reads `screens.json`, `journey-catalog.md`, verification
   checklists, bug triage, and the README coverage table. It emits deterministic
   browser data and fails on duplicate, missing, or unmapped IDs.
3. `app/` is a dependency-free static explorer with overview, screen, flow,
   and gap views. Hash routes make individual screens and journeys linkable.
4. `capture/` records how visual artifacts are produced and what source state
   they represent. Product-source fixtures are allowed only for states that
   cannot be reached safely without external credentials or destructive data.
5. Repository checks validate the derived data, local links, screenshot
   provenance, and Storybook impact for changes to `tonk-ui` or `tonk-cli`.

## Initial visual coverage

- Browser shell: boot, Hub, space home and navigation, join/share/sync
  ceremonies, account choice/create/login, CLI handoff, account and device
  settings, destructive review, activation success and failure.
- CLI: discovery/help, error output, space inventory and lifecycle, account
  status/login/devices, sync, collaboration, authoring, evaluation, inspection,
  blobs, transfer, and maintenance.
- All 78 existing journey IDs must resolve from at least one screen family.

## Verification

- `python3 docs/storybook/scripts/build.py --check`
- `python3 docs/storybook/scripts/check-links.py docs/storybook`
- `git diff --check`
- Serve the repository over HTTP and inspect the explorer in isolated Chrome:
  desktop, compact viewport, keyboard navigation, accessibility snapshot,
  console, and network requests.
- Run the narrow Rust/CLI checks needed by any capture helpers or source changes.

## Deliberate boundaries

- This slice does not mark the source-derived feature documents `verified`.
  Runtime screenshots prove appearance at one state, not the interrupt and
  recovery matrix.
- The explorer is a local repository tool and is not packaged into Tonk's
  deployed Cloudflare assets.
- Screenshot diffs are review evidence, not automatic product correctness.

## Follow-up polish

- [x] Replace the ornamental card treatment with a flat, grid-led visual system.
- [x] Remove links that only open repository Markdown from the product map.
- [x] Recapture `WEB-02` from the populated Hub route and verify it is distinct
  from `WEB-03`.
- [x] Reject byte-identical screen artifacts during the Storybook build.
- [x] Recheck desktop, compact, navigation, detail, role, and gap views in an
  isolated browser.
- [x] Mirror the `rust/tonk-ui` palette and shipped Tonk logo assets.
