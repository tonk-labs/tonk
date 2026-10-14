# Desktop recovery on main

Recovered only tracked `desktop/` from `28f1f72212ee693def902113ff6092e2d244511a`
onto main `55af716ef5d4c026a1578c4dc4b06a8067225133`. The original standalone
import was `4d09512251af6471fa914e3748ac4e4edeafcfe1`, introduced by `94eb307310`.
The source worktree and its index remain untouched. The eight-file activity diff
was backed up separately and recovered in its own tested increment.

No old Rust, core MCP implementation, browser assets, Nix lockfile, caches,
credentials, signing profiles or user data were imported. Desktop's existing
Swift Package.resolved is retained. Production RuntimeLocation identifiers and
storage locations are unchanged.

`mcp/scripts/sync-desktop.mjs` projects current canonical catalog and guides and
bundles `mcp/browser.mjs`. Desktop owns its private transport, inspection and
space-proposal controls. Retired preview/apply/CLI/metadata tool calls are rejected
without execution. Historical text remains readable. New native creation
proposals retain their receipt context and explicit acceptance; submitted proposals
only check their original receipt and never resubmit. Legacy proposals without the
current protocol marker remain saved but require dismissal and a fresh proposal
before any new operation. Metadata/schema reads and
renames use canonical query/evaluate notation.

See `plans/009-rescue-desktop-on-main.md` for current evidence and limitations.

Historical experiment Rust sources and patches are omitted from recovery; they
are not prerequisites and remain available in the source commit.
