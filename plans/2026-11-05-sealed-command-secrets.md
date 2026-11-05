# Sealed command secrets

Status: implemented, on top of command invocations
(`plans/2026-11-05-command-invocations.md`).

## The problem

Invocation tracking copies a followed command's fields into the session
overlay of the branch it ran on. Every view on that branch reads the overlay,
and the overlay is exported to the next worker. Some fields are bearer
secrets: `tonk/join`'s url (the invite's seed rides the fragment),
`account/finish-sign-in-via`'s url (the grant rides the fragment), and an
email invitation's delivery id.

## The design

- **`as: secret`.** An attribute declared so is stored as `bytes:`; the
  analyzer records it as sealed with `tonk.dialog.attribute/sealed: true` on
  the attribute entity, kept out of the descriptor (like `role:`), so the
  attribute's identity is that of the same attribute `as: bytes`.
- **Sealing** reuses `tonk_identity::sealed` (X25519 + HKDF + AES-256-GCM),
  generalised to plaintext of any length under a binding.
  `tonk_identity::secret` adds the field format, `recipient (32) ‖ blob`,
  bound to the attribute it was sealed for, and the key.
- **The key** is derived from the operator: the operator key's own
  agreement-based derivation, then HKDF to an X25519 key. The operator is the
  same for a profile on an access branch across boots and renewals, so the
  key survives a worker restart. Its public half is published on the overlay
  of the profile's meta branch (`tonk:command-secret`).
- **Invokers seal.** The host's `tonk-claim` handler is where every page
  claim converges (a view's `on:` event through `tonk-display`, a portal's
  `window.tonk.transact` through `tonk-portal`), so it seals there: a field
  typed as bytes, bound to text, whose attribute is recorded as sealed.
  `tonk-ui`'s direct profile transacts go through the same function. The
  evaluator (`tonk eval`, the worker's `/evaluate`) seals text a document
  binds to a sealed attribute, to the evaluating operator's key.
- **Fail closed.** `/transact` refuses text in a sealed attribute. The
  dispatcher verifies every sealed field before any provider runs; a field
  that is text, malformed, sealed for another key, or does not open refuses
  the command with a specific error, recorded on its invocation when
  followed.
- **Rust.** `Sealed<T>` is a scalar stored as bytes, with `Debug` printing
  `[sealed]`; `CommandEnv::reveal` opens it.
- **Migration.** The sealed attributes are new relations, so a plaintext
  value on them can be refused while the plaintext shapes on the old
  relations still run. Those run through their own providers (a plaintext
  url cannot convert into a sealed one), and an invocation copies only the
  current shape's attributes, never `secret::PLAINTEXT_LEGACY`.

## Where this departs from the brief

- **Device authorization carries no secret.** `tonk:authorize-device`'s
  audience is a public DID and its callback a loopback URL; the CLI says as
  much (`tonk-cli/src/account.rs`, `login_url`). `tonk:sign-in-via`'s `via` is
  an origin. Neither is sealed.
- **The CLI does not dispatch commands.** `tonk eval` evaluates into its own
  site and runs no provider. "The CLI seals" is the evaluator sealing to the
  site's operator key, which is also what makes an agent's `/evaluate` seal.
- **The host, not only the portal.** Sealing only in `tonk-portal` would miss
  every declarative view, `/join` among them.
- **Which attributes are sealed** is recorded by the analyzer and read by the
  host, as agreed; the worker and the evaluator check a static list
  (`tonk_schema::secret::SEALED`) instead of querying the branch on every
  dispatch. A test keeps the list and `profile.yaml`'s `as: secret`
  declarations in step. An attribute a user declares `as: secret` is sealed
  by the host, but nothing reveals it.

## Known gaps

- **Replay.** A view on the branch can read a sealed blob from an invocation
  and send it again; the worker opens it. Sealing stops reading, not replay.
- **Text through a variable or a domain head** in a document is not sealed by
  the evaluator; the worker refuses it at dispatch.
- **A key that cannot be derived** (a non-Ed25519 operator, a platform crypto
  failure) publishes no recipient, so every sealed command fails closed.
- **Bundle size.** `tonk-host` now depends on `tonk-identity`.
