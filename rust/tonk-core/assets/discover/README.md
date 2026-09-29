# Vendored Honky Tonks templates

Source: https://github.com/goblinoats/honky-tonks
Revision: `6468d614151832ac1614b470003db4baf7877107`

`catalog.json` preserves the five upstream manifests and records SHA-256 hashes
for the vendored files. `templates/` contains unchanged required application
files, manifests, preview images, and standalone license notices. Embedded
third-party notices remain in the application files. The upstream repository's
MIT license is in `LICENSE`; each template declares its own license.

Run `node scripts/build-discover.mjs` from the repository root to verify hashes
and regenerate `seeds/` and the marked Hub cards in `library/profile.yaml`.
Seeds concatenate required files in manifest order and apply the manifest's
entrypoint as the new space's home. Starter space already defines its home.
Kanoodel and Welcome repeat the standard `component` declaration; generated
seeds omit its redundant `&component` anchor so combined seed validation uses
core's name without a duplicate-name error. Its entity and descriptor are kept.
No application file is reserialized. Optional Nightsky demo media and the
standalone relay development projects are not vendored or installed; its required
core carries its own setup instructions and relay sources.

To update, review a new upstream revision, replace the vendored files and catalog
metadata/hashes together, regenerate, and run the seed and browser checks. This
is a pinned release asset, not a live fetch of the upstream catalog. Trunk serves
it at `/discover/`; the worker fetches a seed only after the user submits.
