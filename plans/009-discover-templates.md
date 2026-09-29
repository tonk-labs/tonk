# Discover templates in the Hub

- [x] Vendor a pinned Honky Tonks catalog with original sources, previews, and licenses.
- [x] Add Discover collection tabs and in-page preview/copy dialogs to the Hub.
- [x] Verify seed compatibility, creation, failure recovery, and responsive rendering.

## Implementation

Upstream: `https://github.com/goblinoats/honky-tonks` at
`6468d614151832ac1614b470003db4baf7877107`. Five templates are bundled in
`rust/tonk-core/assets/discover`. The checkout was clean; URL seeding (#1022)
and duplication (#1029) were already present.

`node scripts/build-discover.mjs` verifies the source hashes and generates seeds
and the marked Hub cards. Required files retain manifest order. Each new space
opens at its template's entrypoint. Starter space already supplies its home;
other seeds receive the CLI's home recipe. Kanoodel and Welcome repeat core's
`component` anchor, so generated seeds omit that redundant anchor while keeping
its entity and descriptor. Original vendored files remain unchanged. Optional
Nightsky demo media is excluded; its dialog explains the relay requirement.

The Hub reuses `space-create`, creation receipts, busy/error/retry behavior, and
existing card frames. Bundled seed URLs resolve against the host's origin because
the Hub runs in an opaque-origin guest. Templates appear exclusively on Discover, including for empty profiles. Both tabs sit
inside the wireframe collection panel; cards use its divided author/open footer.

## Verification

- All 53 `tonk-worker --test standard_library` tests pass, including parsing and
  analyzing all five seeds against the current core library.
- `it_creates_spaces_from_all_vendored_discover_seeds` passes: all five templates
  go through real native `space/create` and report successful creation.
- Nine Node tests pass: vendored hashes/generated assets, relative seed URL
  resolution, duplicate-submit prevention, receipt and transport failure recovery.
- Fresh Trunk build passes. Final YAML/CSS assets were copied and restamped with
  the repository packaging script; initial tested artifact: `6d566228ae5a6812`.
- Chrome 154 with matching ChromeDriver passes the new Discover regression:
  cancel creates nothing, missing seed creates nothing, retry creates exactly one
  space, and its Kanoodel home renders. Existing Hub duplication and collection
  card creation regressions also pass against the initial artifact.
- Running-product visual checks pass at 1200 × 900 and 390 × 844, including the
  mobile copy dialog. Captures: `/private/tmp/tonk-discover-desktop.png`,
  `/private/tmp/tonk-discover-mobile.png`, `/private/tmp/tonk-discover-dialog.png`.
- Formatting, tracked diff whitespace, generated catalog consistency, Storybook
  generation, and 177 local Storybook link checks pass. SPACE-15 and WEB-02 updated.

Resolved check failures: duplicate upstream anchors; a missing description in the
initial generated home recipe; a test helper that only lowers concept claims
instead of exported domain facts; and a browser test whose injected hidden-input
URL survived form.reset(). Visual inspection caught inherited nowrap styles and
a mobile menu column; template-specific styles correct both. No worker runtime
or duplication logic changed.

Not run: full workspace suite, hosted/cross-device sync, Safari or physical-device
checks, and all internal interactions in every community template. Existing
creation behavior still applies if storage fails after space allocation or a
response is lost. The vendored catalog is updated by review, not fetched live.

## Wireframe correction

Removed the empty-profile inline Discover fallback and introductory heading.
Copied the joined icon tabs, panel padding, and desktop/mobile card footer layout
from the supplied wireframe. The browser regression now checks initial hidden
templates and both directions of tab switching before exercising copy/retry.

Final correction artifact: `4e4d241b51d993ed`. Focused Chrome tab/copy regression,
nine Node checks, formatting and diff whitespace checks pass. Desktop panel and
mobile compact cards inspected in isolated Chrome. Asset stamping initially raced
a local dist staging update; rerunning after staging completed succeeded.

## Two-step template flow

The card opens details with description, author, licensing and an expandable photo.
Make a copy closes details and opens a separate name/description form using the
existing creation flow. Back to details closes the form and restores details.
Photo expansion uses a native modal above details; closing it returns to details.
The focused browser regression exercises the separate steps and image expansion.

Two-step validation: Chrome regression passed against isolated artifact
`a9d9e7a1aafbd4d4` (11.94s), covering hidden form fields on details, image
expansion/close, back navigation, failed seed recovery and exactly one working
copy. Details and copy dialogs were visually inspected at 1200x900 and 390x844;
Escape from expanded photo restores focus to its opener. Nine Node tests,
formatting, generated catalog consistency, Storybook build and 177 links pass.
