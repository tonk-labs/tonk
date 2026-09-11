# Bundled Welcome content

`export.py` converts the product-wip CSV export to a complete typed JSON
snapshot. `playground.py` adapts the existing Agent playground prompt.
After applying any asset compression, run:

```sh
B3SUM=b3sum python3 scripts/onboarding/defer.py /tmp/full-onboarding.json
```

The splitter produces `onboarding.yaml` (Welcome/shell and sidebar metadata),
`onboarding-demos.yaml` (one optional shard), `onboarding-media.json`, and two
content-addressed lossless WebP files under `rust/tonk-core/assets/library`.
The optional shard never contains an initial shard's (attribute, entity) pair.
The seven initial module identities and Vault shell roots are explicit;
compiled rule references, attribute definitions, aliases and induced rules
extend that closure. This is a splitter for this authored starter application,
not a general application dependency analyzer. Recheck all page navigation when
changing its roots or authored dependencies.

The loader is maintained in `deferred.js`, appended to the Vault active module
by the splitter. Running without an input merges the committed partitions and
regenerates them deterministically. BLAKE3 is needed only when extracting new
inline images. The Rust native fixture's `fetch_media` allowlist must also be
updated along with the Trunk copy declarations in `rust/tonk-ui/index.html` if image identities change; worker tests verify its bytes and hashes.

Welcome returns before optional import. Two animation frames after real Welcome
content appears, the loader starts optional preparation; earlier navigation
awaits the same request. Images use IntersectionObserver and the existing host
fetch relay to the branch blob route. Optional preparation also persists their
bytes for sync/export and offline use. Browser generation caching may download
binary assets in the background independently of their branch import.

Each shard's marker commits atomically with its facts. The credential journal
records Welcome readiness and eventual completion separately. Retrying an
imported shard never reasserts its data, and the first optional import preserves
all already-existing attribute/entity pairs. Sidebar edits and removals are
not part of the late shard. A generation-pinned worker asset reader supports
interrupted preparation after the complete offline generation is cached.
