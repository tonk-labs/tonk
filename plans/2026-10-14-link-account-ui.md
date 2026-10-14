# Link account UI refresh

Approved reference: prototypes/link-account/link-account.html, revised from
/Users/jackdouglas/tonk/gooey/fabb v0.18.0 (not the older monorepo DESIGN.md).

Scope: signed-out connection registration, activation page, and redundant email
reminder. Reuse the current contained account presentation without changing its
navigation ownership. Preserve signup/passkey/activation ordering: the real
service sends activation after registration, unlike Gooey's simulated email-first
verification. Preserve terms acceptance and the original link destination.

- [x] Apply shared rounded task styling to the unanchored link account dialog.
- [x] Put verification resend inside the dialog and suppress the background toast.
- [x] Restyle standalone activation with the same shell and footer.
- [x] Verify source fixtures at desktop/mobile, focused native tests, Wasm check and format.

PR requested after implementation review. No deployment requested. Live email/passkey delivery needs separate proof.

Validation complete: formatting/diff checks pass. Source fixtures inspected in
light desktop and dark mobile; 320x260 footer remains visible with scrolling body.
The hub is an opaque guest, so notice suppression belongs in profile.yaml and
uses its /settings/link route, not a top-document :has(dialog) selector.
Native register_dialog tests: 12 passed. Wasm check passed after correcting a
missing borrow caught by the first check. Existing unrelated dead-code warnings
remain. Extended service-backed integration and Wasm DOM tests were not executed:
this checkout has no built UI deployment. Live passkey/email/cross-tab behavior
and production rendering remain unverified; fixtures prove presentation only.

CI follow-up: MCP run 38560408358 failed before worker tests because
ptr_meta/example.md disappeared from ~/.cargo. The runner logs show
nothing-but-nix deletes that directory in an asynchronous cleanup. Move this
job's CARGO_HOME to /nix/var/tonk-mcp-cargo alongside its existing target directory
and create both with runner ownership before entering the Nix shell.
