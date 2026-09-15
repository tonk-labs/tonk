# Space login recovery

## Scope and coverage

Missing-space screen → existing account ceremony → original space. Implemented
in Rust custom elements and the profile YAML view, using the existing portal
registration bridge and edge styles. Read the supplied AGENTS.md instructions
and DESIGN.md. No additional AGENTS.md applies to these source directories.

Both missing-directory fallbacks now offer sign-in and invite recovery. The
separate downloading state remains. The top page captures its own space URL,
including query and fragment, and keeps it throughout the account ceremony.
A profile-binding reload therefore opens the same space. No redirect is stored
in session storage. Ordinary login still returns home; sharing still uses its
existing pending-share path.

| Domain | Evidence inspected | Result |
| --- | --- | --- |
| Accessibility | Native button/link names in Chrome accessibility snapshot, keyboard focus, 44px button, reduced-motion and forced-color rules | Clear within inspected scope; VoiceOver and OS high-contrast rendering not verified |
| Layout | Extracted production markup/CSS at 320px and 500px, 200% CSS scaling, scroll-to-action | Zoom clipping fixed; no horizontal overflow |
| Writing | Both absence states, downloading state, dialog heading/status and existing email/passkey copy | Misleading denial fixed; sign-in and invite recovery remain distinct |
| Typography | Rendered real Plex fonts, wrapped copy at 320px and 200% scaling | Clear; existing compact type system retained |
| Colors | Computed text/background pairs in both themes | Body 7.71:1 light / 10.16:1 dark; primary 14.60:1 / 12.65:1, all above 4.5:1 |
| UI | Existing square controls, primary/quiet hierarchy, focus appearance, declared hover/press/reduced-motion behavior | Clear within inspected scope; slow-motion replay not verified |

The visual preview extracted the production recovery markup and CSS and used
repository font/image assets. It was not a full deployed app or an installed
PWA. Browser unit tests separately exercised the compiled Rust components and
navigation. Existing account dialog loading/error behavior was inspected in
source, not replayed with a live passkey provider.

## Findings addressed

| Severity | Domain | Location | Before | After | Why |
| --- | --- | --- | --- | --- | --- |
| HIGH | Writing | `rust/tonk-core/assets/library/profile.yaml:1185`, `:1199` | “you don't have access”; home was the only action | “open this space”, plain Home Screen explanation, direct “sign in” button, separate invite guidance | Missing local state does not prove that access was denied |
| HIGH | Layout | `rust/tonk-core/assets/library/profile.yaml:1072` | Fixed inset panel also had `min-height:100vh/100dvh`; at 200% scaling its bounds were 1136px in a 568px viewport | Remove the redundant minimum height; panel is 568px and the button scrolls fully into view | Recovery must remain reachable when enlarged |

## Verification

Passed:

- `cargo fmt --all -- --check` and `git diff --check`.
- `cargo test --locked -p tonk-worker --test standard_library`: all 32 tests on final source, including library lowering and both recovery states.
- `cargo test --locked --target wasm32-unknown-unknown -p tonk-ui -p tonk-workspace --lib space_login`: 3 browser tests.
- `wbg-pool target/wasm32-unknown-unknown/debug/deps/tonk_ui-e372ad03b9ba694a.wasm`: all 18 UI browser unit tests, including pending-share contracts.
- Isolated headless Chrome: local preview, `emulate --viewport '320x568x2,mobile,touch'`, keyboard Tab to sign in, light/dark screenshots and computed contrast.
- At 200% CSS scaling, scroll the focused button into view: final button bounds y=239.66–327.66 in a 568px viewport, no horizontal overflow. This is CSS scaling evidence, not Safari pinch-zoom evidence.

Initial failures resolved: old wording assertion updated to require both recovery
paths; custom-element required `inject_children` implemented without replacing
authored content; zoom overflow fixed. Localhost/browser startup required sandbox
escalation. Existing dead-code warnings in tonk-worker and tonk-fab remain.

Not verified: installed iOS PWA, real passkey login, full two-device sync journey,
VoiceOver, runtime forced-colors mode, Safari pinch zoom, deployment, CI.

## Verdict

Approve for the inspected recovery screen and browser component scope. Device
and full authentication verification remain separate from this interface review.
