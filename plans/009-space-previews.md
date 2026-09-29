# Cached blurred space previews

Status: implemented and verified locally.

Use the inexpensive DOM/stylesheet capture proven in `bench/space-preview`.
Capture only an already-open space home after it settles; never mount spaces
from hub cards. Bound traversal, image size, refresh rate and cache size. Abort
capture when busy/hidden or when the view exceeds its budget. The current
decorative card remains the fallback. Blur cached images in the hub.

The existing authenticated guest bridge must bind preview writes to the
portal's actual space. Only the profile shell may retrieve multiple spaces.
Keep previews local to the browser tab and profile branch, with bounded age;
account changes must not expose the preceding account's cache.

Checkpoints:
1. Add capture/cache bridge with focused authorization, budget and lifecycle tests.
2. Connect home capture and hub display; verify fallback and blurred rendering.
3. Run relevant formatting, core/portal checks and isolated browser checks.

Do not claim real-space/browser integration or mobile performance from the
synthetic fixture. Record exact completed checks and remaining limits here.

## Implementation

- `tonk-portal` injects a best-effort capture into existing guests. The profile
  marks only the space home as eligible; nested content inherits that identity.
- Capture starts after four seconds, during an idle callback, and checks again
  once a minute. It skips hidden/busy/loading views and wrappers with iframes.
  DOM traversal is capped at 600 nodes and eight milliseconds; a final
  preparation check caps serialization at twelve milliseconds. These checks
  are cooperative guards, not a hard real-time guarantee for individual DOM
  or canvas calls. CSS, source markup, canvas sizes and output bytes are capped.
- Snapshot construction uses an inert document, does not re-run custom element
  constructors, and strips XML-invalid internal attributes. Fonts and external
  resources are omitted rather than fetched. Inputs/editable content are omitted.
- Cache writes use the existing authenticated message port. Every nested
  boundary checks the real home identity. Only the profile can read the cache.
  The top page owns a sessionStorage cache scoped to its profile branch: 32
  images, 24-hour expiry, one write per space per minute, memory fallback when
  storage is unavailable. Switching profile scope discards the prior cache.
- Cards lazy-load 256 by 160 WebP images, use a 6px blur, and retain decoration
  until decode succeeds. A separate pending attribute preserves the image's
  layout box for lazy loading; native `hidden` prevented decoding in the app.

## Evidence

- Wasm portal check passed; full Trunk build passed.
- 170 JavaScript tests passed; focused preview tests passed after the final
  loading-state change (authorization, throttle, cache cap/expiry/account
  separation, reload, storage failure, fallback and reused-card identity).
- 52 standard-library tests passed, including profile parse/analyze/lower.
- Isolated Chrome 154: production capture/cache in an opaque iframe, including
  the app/WA stylesheets, exported successfully. A custom-element constructor
  ran once, not again during capture.
- Real locally built app: created a disposable space and replaced its blank
  home with a garden view through the normal evaluate path. Clearing the cache
  and opening the home produced an automatic 1,863-character WebP data URL.
  Returning to the hub showed a decoded 256px image with `blur(6px)` and zero
  nested space iframes. A diagnostic capture took 24ms end-to-end; this includes
  asynchronous decode/encode and is not a blocking-time measurement.
- Actual app testing caught two gaps in the spike: Tonk's internal control
  characters broke SVG XML parsing, and `hidden` prevented lazy image loading.
  Both were fixed at their failing boundary.
- Final build: `c90b22840e4f2c73`. Desktop 1200x900 and narrow 390x844 were
  visually checked; images remained blurred and the card layout stayed intact.
  Screenshots: `bench/space-preview/hub-preview.png` and
  `bench/space-preview/hub-preview-mobile.png`. Final computed image state:
  decoded width 256, pending false, blur 6px, object position 50% 0%; the hub
  guest had zero child iframes. Formatting and diff checks passed.

## Limits

Previews are local to the tab, appear only after a home has been visited, and
may be stale. Large/unsupported views retain the decorative fallback. No
server-side capture, background space mounting or new dependencies. Firefox,
Safari, physical mobile-device performance, complex media/shadow fidelity,
and hosted CI remain unverified. Blur is presentation, not redaction of the
locally cached image. The browser smoke check is not an automated E2E suite.
