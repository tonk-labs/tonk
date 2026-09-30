# Space thumbnail feasibility spike

## Production capture regression

Serve the **repository root** with
`python3 -m http.server 8772 --bind 127.0.0.1`, then open
`http://127.0.0.1:8772/bench/space-preview/capture-regression.html` in an isolated
test browser. After about 20 seconds, `window.done` is true and every entry in
`window.results` must have `passed: true`. The fixture runs the current production
script unchanged in opaque iframes, with real SVG decode and WebP export.
It covers hidden vault panels, component code, oversized embedded assets and
duplicate runtime CSS, and verifies visible-node and unique-CSS limits remain
enforced. This is a browser regression fixture, not part of the Node test suite.

## Original spike

2026-09-29. Conclusion: cheap cached thumbnails are plausible for simple DOM
views; arbitrary Tonk space capture is not yet proven. No production code changed.

## Reproduce

Run `python3 -m http.server 8769 --bind 127.0.0.1 --directory bench/space-preview`
from the repository root, then open `http://127.0.0.1:8769` in a test browser.
Results appear below the source iframe and are available as `window.results`.
The fixture automatically captures six times per method, then three times per
method for a denser view. No dependencies or privileged screenshot APIs are used
to generate thumbnails. The screenshot artifact only documents the outputs.

## Boundary and measurements

The iframe copies the sandbox flags from `rust/tonk-portal/src/shared.rs`:
`allow-scripts allow-forms allow-downloads`, without `allow-same-origin`.
Access to the parent document correctly throws SecurityError. Capture executes
inside the guest, serializes DOM into SVG foreignObject, draws it onto canvas,
and exports a 320 by 320 WebP at quality 0.75. The capture viewport is 1000 by
1000, deliberately independent of the visible 1000 by 600 iframe viewport.

Local headless Chrome 154, no CPU throttling. Final run in `results.json`:

| Method | Elements | Synchronous preparation | Total elapsed | WebP size |
| --- | ---: | ---: | ---: | ---: |
| DOM + stylesheet | 58 | 0.1–0.2 ms | 8–12 ms | 8,174 B |
| All computed styles | 58 | 28–31 ms | 38–52 ms | 11,074 B |
| All computed styles | 410 | 188–196 ms | 220–269 ms | 13,344 B |
| Stylesheet + selective state copying | 58 | 1–1.6 ms | 8–9 ms | 8,584 B |
| Stylesheet + selective state copying | 410 | 1.5 ms | 8–11 ms | 10,738 B |

Total includes asynchronous image decode and encoding; it is not all blocking
time. Samples reuse identical content and benefit from browser caches. These
are feasibility measurements, not production latency percentiles. Long tasks
were observed during the expensive computed-style experiments; the fixture is
not an interaction responsiveness benchmark.

## Findings

- Blob-backed SVG taints the canvas in this fixture; all six export attempts
  threw SecurityError. Data-URL SVG exported successfully without relaxing the
  iframe sandbox.
- Plain DOM cloning loses shadow content, canvas pixels and edited input state.
- Selective copying restores these fixture cases cheaply: flatten the simple
  open shadow root, substitute canvas pixels, copy current input value, and
  retain the document stylesheet and body inheritance.
- `captures.png` was visually inspected. Top row: plain, computed-style,
  selective. Bottom row: dense computed-style and selective. The computed-style
  method loses the ancestor background; the selective method preserves it.
  None is a general-purpose screenshot implementation.

## Limits and next checkpoint

This uses synthetic content, not a running Tonk space or the guest Wasm bundle.
It matches the portal's opaque-origin boundary only. Tonk's main display/view
elements use light DOM, but nested portals remain separate opaque iframes.
The fixture does not capture those nested documents. Shadow style scoping,
slots, adopted stylesheets, blob fonts/images, external assets, video, WebGL,
scroll offsets and cross-browser behavior remain unverified. No cache/storage
or hub rendering benchmark was implemented. No claim of imperceptible overhead
on mobile devices is justified yet.

Next checkpoint: capture one representative real home inside its existing
guest, including its assets and nested portal, while measuring input/frame
latency. Avoid walking every computed style. If that succeeds, generate only
opportunistically while a space is already open, cap capture work, and cache
the image per account/space. The hub should only read cached images and never
mount spaces for thumbnail generation. Remove cached private previews when
their account/access is removed. Do not extend this fixture into production
without validating fidelity and cleanup at that boundary.
