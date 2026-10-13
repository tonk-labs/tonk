# Proof of concept: the pin on a branch's head

Not for merging. This branch builds against a patched Dialog through the
`[patch]` table at the end of `Cargo.toml`, which names a clone on the
machine it was made on.

`dialog-boot-head.patch` is that patch: two commits on Dialog `120fba8`
(tag `tonk-2026-10-03`), which is what tonk builds against. To rebuild the
clone elsewhere:

```sh
git clone https://github.com/dialog-db/dialog-db.git dialog-boot
cd dialog-boot
git checkout -b poc/boot-head 120fba8
git am /path/to/tonk/plan/poc/dialog-boot-head.patch
```

Then point the `[patch]` entries in `Cargo.toml` at it and run
`cargo update -p dialog-capability`.

## What it shows

A branch's head carries a boot statement: a line of releases and the release
pinned from it, signed by whoever set it and not covered by the head's own
signature. A reader from before the field decodes and verifies such a head
as it always did.

- `tonk:pin-release` on a branch has the worker that holds it sign a
  statement, set it on the head, and record the pin in the branch
  (`rust/tonk-worker/src/router/boot.rs`).
- A commit carries the statement forward, a push publishes it, a pull adopts
  it, a merge keeps the later.
- Wherever a head lands, the worker writes a verified pin where its origin's
  shell reads it, and the shell registers the bootloader for that release.

## What it does not do

- It checks that a statement is signed by the key it names, not that the key
  may decide what the branch runs.
- It does not put a statement back on a head minted by a writer from before
  the field.
- It does not hand the pin to a new device with its delegation.
- Nothing in the interface fires the command.
