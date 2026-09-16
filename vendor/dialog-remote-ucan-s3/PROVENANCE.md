This directory vendors only `dialog-remote-ucan-s3` from Dialog revision
`4c16de9e345d2b2d888d1008c5d3f0ca990c4807` (MPL-2.0; see LICENSE).
The sole source change is `patches/dialog-transport-expiry.patch`: ordinary
verified-chain S3 signing is limited to 60 seconds and remaining ancestor
validity, with a signing-time clock recheck. No connection checkpoint is added.
Unsigned/public endpoints cannot enforce URL expiration.

Reproduce with `scripts/test-transport-expiry.sh --prepare-only`, then run
`python3 scripts/sync-dialog-transport-vendor.py DIALOG_CHECKOUT --check` against
the printed patched Dialog checkout. Omit `--check` to regenerate. The script
expands the original upstream workspace manifest and points all other Dialog
crates at the unchanged git revision. Replace this vendor with an upstream pin
once the isolated expiry change is published and verified.
