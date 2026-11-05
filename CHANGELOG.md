# Changelog

## Unreleased

### A command's secret fields are sealed to the worker that runs them

Design note: `plans/2026-11-05-sealed-command-secrets.md`.

An attribute can now be declared `as: secret`. Its value is stored as bytes
sealed to the worker (X25519 + AES-256-GCM, `tonk_identity::sealed`), never
as the plaintext. The commands that carried a secret in plain text now carry
it sealed: `tonk/join`'s url, `account/finish-sign-in-via`'s url, and the
delivery id of `tonk:send-email-invitation` and `tonk:claim-email-invitation`.
Invokers seal before sending: the host seals every claim a page makes (a
view's event or `window.tonk.transact` in a portal), and `tonk eval` and the
worker's `/evaluate` seal what a document binds. Page JS and YAML views need
no crypto. Only the provider reveals the value, so it no longer sits in plain
text in the session overlay an invocation keeps, nor in the overlay handed to
the next worker.

What this changes for you:

- New notation type: `as: secret`. It lowers to `bytes:` and records the
  attribute as sealed (`tonk.dialog.attribute/sealed: true` on the attribute
  entity). The same attribute declared `as: bytes` is the same entity.
- New attributes: `xyz.tonk.command.join/sealed-url`,
  `xyz.tonk.command.finish-sign-in-via/sealed-url`,
  `xyz.tonk.command.send-email-invitation/sealed-id`,
  `xyz.tonk.command.claim-email-invitation/sealed-id`. `profile.yaml`
  declares the join and finish-sign-in-via commands with them, and now
  declares both email invitation commands (`email-invitation/send`,
  `email-invitation/claim`), which had no declaration.
- Rust: `tonk_schema::command::{Join, FinishSignInVia, SendEmailInvitation,
  ClaimEmailInvitation}` hold their secret as a field whose value is
  `tonk_schema::secret::Sealed<String>`. A provider reads it with
  `CommandEnv::reveal(&field)`. `Sealed`'s `Debug` prints `[sealed]`, which
  replaces the hand-written `[redacted]` impls on these four types.
- The worker publishes the key to seal to on the overlay of the profile's
  meta branch: `tonk:command-secret`, `xyz.tonk.command-secret/recipient`, a
  `did:key:z6LS…`. It is derived by HKDF from the operator key, so it is the
  same across worker restarts and session renewals, and changes with a
  profile or account switch.
- Plain text where a secret belongs is refused before anything commits:
  `/transact` answers 400 with "the secret was sent without being sealed",
  and `/evaluate` or `tonk eval` with no key to seal to refuses the
  document. A sealed field that does not open fails the command before any
  provider runs: "the secret was sealed for an earlier session" for a blob
  sealed to another worker key, "the sealed secret did not open" for one
  altered or moved from another field. A followed command records the
  failure on its invocation.
- The plaintext shapes still run, through their own providers
  (`command::legacy::{JoinUrl, Join, FinishSignInVia, SendEmailInvitation,
  ClaimEmailInvitation}`). An invocation copies only the attributes of the
  shape the command has now, and never a secret a legacy shape carried
  (`tonk_schema::secret::PLAINTEXT_LEGACY`).
- `CommandHandler` gained `fields()`, the current shape's attributes; it
  defaults to `trigger_attributes()`.
- `tonk-display` passes text through for a field typed as bytes instead of
  dropping the event, so the host can seal it. Text the host does not seal
  reaches the worker, which refuses it.

Why: command invocation tracking copies every field of a followed command
into the origin branch's session overlay, where every view on that branch can
read it and from where it is exported to the next worker. A join url's
fragment, a sign-in grant and an email delivery id are bearer secrets.
Sealing them at the invoker keeps the record of a command without keeping
what it unlocks.

### Compatibility and migration

- Commands are transient, so no replica stores a secret, sealed or not;
  nothing is rewritten on upgrade.
- A profile seeded before this release keeps its plaintext `tonk/join` and
  `account/finish-sign-in-via` declarations, so its pages send the plaintext
  shapes, which still run. Plaintext decoding is to be dropped in a later
  release.
- An invocation an earlier worker kept may hold a plaintext secret in the
  overlay a successor restores. It expires on its retention (at most ten
  minutes by default).
- A worker from before this release does not know the sealed attributes, so
  a sealed command reaching one is unhandled. The host and the worker ship
  together, so this needs a worker older than its page.
- Embedders run nothing.
