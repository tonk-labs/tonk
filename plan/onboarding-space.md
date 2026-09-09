# First visit onboarding space

On the first visit to `/`, the UI asks the worker to initialize a local copy
of product-wip before mounting the router. The worker serializes setup with
profile changes and concurrent tabs, records the created space before seeding,
and records completion only after content is ready. A completed browser returns
no redirect, even after the space is removed. Explicit routes are unaffected.
Existing accounts and profiles with spaces go to the Hub.

The bundled onboarding.yaml is a typed artifact snapshot in YAML's JSON subset,
exported with `tonk --space product-wip export` and converted using
`scripts/onboarding/export.py`. It preserves compiled rules and views while
excluding credentials, governance, history, and the source repository identity.
It is a private local copy, with ownership minted through normal space creation.

Validation completed locally: the worker unit suite passed all 124 tests, and
all four onboarding tests passed again after the final seed change. The UI built
through Trunk for Wasm. The real-browser test
`it_opens_the_welcome_space_once_then_the_hub` passed against that build in
Chrome 152; a separate isolated browser check confirmed the second visit shows
the Hub with exactly one welcome space. Formatting and diff checks passed.

The exported `vault-active` component contained a stale `autoOpen` reference
that prevented the configured welcome page from opening. The export converter
removes that condition in the bundled copy; the original local space is unchanged.
Both referenced saved-game blobs are bundled with the snapshot. The converter
accepts their `blob:<hash>=<local-file>` mappings as positional arguments.

The full account/browser suite, Safari, and hosted deployment were not run.
No commit or deployment was requested.

The welcome endpoint is a pre-mount bootstrap probe: it must finish before the
router stamps a site or opens its first subscription. This follows the bootstrap
exception in `.claude/skills/commands-not-routes/SKILL.md`; it is pinned in the
route-table test. Ordinary space creation remains the existing command.

The Agent playground now uses the standard empty-space account-aware `connect`
prompt, adapted to retain the existing playground page and navigation. Its copy
value explicitly limits changes to that page's own views, components, and data,
and asks before expanding scope. The old manual share/join instructions are
replaced with the standard handoff state and copy control. This is an instruction
scope; it does not add page-level capability restrictions.

`scripts/onboarding/playground.py` derives `onboarding-agent.yaml` from the
current core prompt. Run it after changing the standard prompt. Its independent
`onboarding/agent-invite` model avoids the imported legacy invitation schema.
The export converter also adapts the playground view, so re-exporting preserves
this integration. The worker seeds this small library after the snapshot.

Playground validation: all four focused worker tests passed with the new
handoff-to-prompt rule assertion. The extended Chrome test passed at 1200×900,
checking the account-required state, injecting a prompt fixture, clicking the
actual copy control, and asserting the copied connect command and page scope
before returning to the Hub. The UI Wasm build and deterministic regeneration
checks passed. Full account authorization and CLI round-trip were not rerun.

Playground layout polish: the exported view now gives the account-required and
ready-prompt states one padded card with a heading. Status and card spacing share
consistent insets; the page retains side gutters at narrow widths. Removed the
obsolete share/join card CSS. Checked both states in an isolated Chrome browser
using the exact generated view, including desktop and narrow layouts; no Rust
behavior changed. The export adapter remains idempotent and diff checks pass.

The presence row now aligns its text on the baseline and removes the dot's
old relative vertical offset. Verified the generated view in isolated Chrome;
`git diff --check` passes.
