# Account linking prototypes — Gooey revision

Approved review prototype. The implementation and validation record are in
../../plans/2026-10-14-link-account-ui.md and validation/README.md.

Source of truth inspected 2026-10-14:
- /Users/jackdouglas/tonk/gooey/DESIGN.md, September 21 updates and review surfaces
- fabb/fabb.js: current task shell and tonk-flow records
- fabb/account-demo.js and fabb/fabb-forms.css
- Rendered fabb/hub.html?flow=signup account form

Replaces the rejected legacy block-cluster prototype. Current treatment: 360px
rounded shell, 25px radius, 1.5px ring, 48px header/footer, 17px semibold controls,
18px regular condensed message text, continuous wash and editable completed rows.
Email-first entry; alex@example.com demonstrates returning-user detection.
New-account sequence follows Gooey: email, verification, name, passkey.

Connection wording and standalone activation are proposed adaptations. Keep
existing explicit terms acceptance pending implementation review. Email completion
is represented by selecting Display name in the external preview picker; there
is no simulated verification button inside the product surface. All passkeys,
emails and approvals are simulated. This does not prove live auth or routing.

Approved by the user; implementation preserves the real service order (passkey
registration before verification email), unlike this simulated sequence.
The monorepo DESIGN.md is older than
the Gooey source and was not updated as part of this focused UI change.
