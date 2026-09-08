# Handoff empty-state fixes

Scope: preserve existing account-matched handoff work; fix the false receipt,
pending prompt with an empty URL, and oversized dismissal control.

Evidence:
- `cargo test -p tonk-render --test compat handoff_`: absent receipt still renders;
  absent prompt does not. Receipt has no subject binding.
- Pending handoff and ready prompt reuse link/account attributes, allowing the
  prompt concept to match pending facts without the ready rule.
- The rule used inductive `assert!`, retaining a ready prompt after source
  invalidation. Changed to deductive `assert` so availability follows live state.
- Isolated Chrome with bundled Web Awesome CSS: close control is 76 by 76px
  inside a 44.84px toast; native disclosure padding, margin and chevron leak in.

Tasks:
- [x] Prove pending response cannot resolve as a prompt; separate derived attributes.
- [x] Bind receipt container to its entity and verify absent/present rendering.
- [x] Direct accountless users through the existing share account action.
- [x] Reset disclosure styling and verify dimensions and keyboard dismissal.
- [x] Run focused checks and record limitations.

Final validation:
- `cargo test -p tonk-render --test compat`: 10 passed.
- `cargo test -p tonk-cli --features integration-tests --test handoff`: 12 passed,
  including unavailable-to-ready-to-invalidated prompt lifecycle.
- `cargo test -p tonk-worker --test standard_library`: 29 passed.
- `cargo check -p tonk-worker --target wasm32-unknown-unknown`: passed (existing
  unused cache helper warnings). Native worker tests also report the browser-only
  handoff function as unused.
- Isolated Chrome, bundled Web Awesome CSS: toast and close control are 44px
  tall; close control is 44px wide, contained at desktop and 375px mobile widths.
  Native Tab/Enter dismissal hides it; mobile dark-mode screenshot inspected.
- Explicit retraction in the invalidation fixture models the worker overlay's
  cardinality-one replacement. This isolated the retained inductive conclusion;
  the prompt now uses a deductive rule. Production overlay behavior is unchanged.
- Touched Rust files pass rustfmt. Workspace-wide formatting/diff checks found an
  unrelated trailing blank line in `rust/tonk-cli/tests/common.rs`; preserved it.
- Full browser signup and hosted agent acknowledgement are unverified. Seeded
  view changes apply to newly created spaces; existing frozen views need an
  explicit library refresh. No local data or existing space views were changed.

## Copy action styling follow-up

- Applied the DESIGN.md primary-action treatment to the existing copy component:
  square ink surface, condensed lowercase label, bottom-right alignment, 144px
  minimum width and 44px hit target, 150ms hover/press transitions and .96 press
  scale. Reduced motion disables transitions and scaling.
- Success and error labels retain the ink palette and stable width. Removed the
  component's icon-pop animation and duplicate accessible label text; retained
  its clipboard handling and feedback reset.
- Verified an actual clipboard copy with the bundled component in isolated
  Chrome, plus error-to-idle reset, a single accessible name, and no horizontal
  overflow at 375px. Inspected desktop and mobile screenshots. This was an
  extracted-component preview, not the running app.
- Final focused renderer test and scoped `git diff --check` pass.

### Alignment and contrast revision

The screenshot review showed the primary fill and bottom-right label were too
heavy for this prose panel. Revised to a secondary frost surface with a subtle
ink border, centered label and symmetric padding. Retained the condensed type,
44px target, stable feedback width and 150ms press transition. Inspected the
updated browser preview and confirmed keyboard copying; both focused renderer
tests pass. The independent toast/button changes are committed separately from
the account-gate fixes, which depend on the larger handoff work. The user subsequently requested that the remaining handoff implementation be committed as well.
