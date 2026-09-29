# Discover templates in the Hub

- [x] Load the live Honky Tonks catalog and previews in production.
- [x] Keep templates exclusively on Discover and match the Hub wireframe.
- [x] Split details/photo preview from copy name and description options.
- [x] Remove all copied upstream sources, generated seeds and catalog generator.
- [x] Verify remote installation, failure recovery and current upstream compatibility.
- [x] Prepare PR #1030 update with runtime catalog integration.

## Implementation

Discover fetches https://goblinoats.github.io/honky-tonks/catalog.json on first
opening the tab. This is the upstream repository's published catalog; no submodule,
GitHub API credentials, copied templates, or build-time catalog snapshot is used.
The UI resolves preview URLs against the catalog, escapes metadata and provides
loading, empty, error and retry states. Revisiting the tab reuses its loaded cards.

Make a copy sends a catalog URL with the selected slug as its fragment. The worker
fetches that catalog and its required files in declared order, verifies SHA-256
hashes, expands scoped includes, supplies the manifest entrypoint as home and
checks the result against core before allocating a new space. Optional files are
not installed. Files must be under the catalog directory, HTTPS (loopback HTTP
for tests), without credentials or redirects, and within the existing size limit.
Kanoodel/Welcome's redundant component anchors and Starter's existing home keep
the compatibility adaptations used by the previous implementation.

The two-step dialogs retain creation receipts and busy/error/retry behavior.
Tests use small Tonk-authored fixtures, not copied community applications. An
opt-in native test checks all current live templates without putting them in Git.

## Verification

Passed: native remote catalog creation and changed-checksum refusal; optional-file
omission; existing URL-seed success and failure regressions; all five live upstream
templates fetched, hash-checked and analyzed; ten Node catalog/form tests; live
browser catalog/previews fetched from upstream with the two-step dialog intact.
Fresh Trunk build passed (artifact `79cfdbd216a68205`). The deterministic
cross-origin browser regression passed in 5.24s, covering catalog failure/retry,
tab exclusivity, photo expansion, separate copy options, missing-source refusal,
retry and exactly one copy whose remote-defined home renders. Formatting,
Storybook generation and all 177 link checks pass. Full workspace, Safari, physical
devices, hosted sync and all internal community-app interactions are out of scope.

Final live browser smoke: fetched little writer directly from the public catalog,
created a fresh local copy and opened its editor (document title/content controls
and word count visible). No production account or shared space was used.
