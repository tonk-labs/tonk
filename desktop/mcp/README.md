# Desktop MCP connection

The implementation, dependencies, tool definitions, guides, and transport tests
live in [`../../mcp`](../../mcp/README.md). These files are compatibility launchers
for external clients. No second npm install or machine-local checkout pointer is
needed. Run `npm ci --ignore-scripts` in the monorepo `mcp/` directory once.

Build with `bash desktop/scripts/build-app.sh` from the monorepo root. The build
regenerates the shared tools, guides and adapters before compiling the app.

Launch with `--mcp-space did:key:YOUR_SPACE` to enable an authenticated loopback
connection pinned to that space. It exposes `tonk_query`, `tonk_guide`, and
`tonk_inspect_view`. Add `--mcp-write` to enable `tonk_evaluate`. The account must
already have access; changing the visible space never retargets this capability.
Inspection requires the pinned space's preview to be open.

Configure the MCP host to run Node with `mcp/server.mjs` (or this directory's
forwarding launcher) and the private connection-file path, normally
`~/Library/Application Support/Tonk/MCP/connection.json`. Local-runtime mode uses
`Tonk Local`. The file grants access; keep it out of logs and agent context.
Restarting the app rotates its bearer token; restart the MCP client afterwards.

Query and evaluate use the shared ordinary-worker adapter and return its unchanged
response. Evaluate executes once; query first after any uncertain write. Neither
an evaluation response nor inspection proves remote synchronization or interaction.
Guide tools and `tonk://guide/<topic>` resources serve the canonical monorepo text.

Run `npm test` in the root `mcp/` directory for transport/contract tests, and
`bash desktop/scripts/dev.sh test:unit` for native tests. A read-only live check
for the disposable checklist fixture is `node mcp/smoke-desktop-inspection.mjs
/absolute/path/to/connection.json`.
