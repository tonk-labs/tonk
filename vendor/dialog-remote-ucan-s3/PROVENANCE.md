This directory vendors only `dialog-remote-ucan-s3` from Dialog revision
`e26e0e61da76e460339c45bd45c4d7e9ce9a801b` (MPL-2.0; see LICENSE).
The sole source change is `patches/dialog-transport-expiry.patch`: ordinary
verified-chain transports are limited to 60 seconds and remaining ancestor
validity, with a signing-time clock recheck. The authorizer exposes the same
absolute deadline so an embedding service can preserve it when translating an
unsigned internal request into its own signed permit. No connection checkpoint
is added.

Reproduce with `scripts/test-transport-expiry.sh --prepare-only`, then run
`python3 scripts/sync-dialog-transport-vendor.py DIALOG_CHECKOUT --check` against
the printed patched Dialog checkout. Omit `--check` to regenerate. The script
expands the original upstream workspace manifest and points all other Dialog
crates at the unchanged git revision. Replace this vendor with an upstream pin
once the isolated expiry change is published and verified.
