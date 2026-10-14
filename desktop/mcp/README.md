# Desktop private connection

Desktop owns its authenticated loopback bridge in
`Sources/HarnessCore/LocalRuntimeBridge.swift`. The native provider integration
uses its `/mcp` endpoint. Query/evaluate execute through the generated
`mcp/browser.mjs` adapter in the account WebKit worker; guide text is canonical.

`--mcp-space did:key:YOUR_SPACE` exposes the private `/tools` and `/call`
transport pinned to that space, with inspection and read-only query by default.
`--mcp-write` grants evaluate. Changing the visible space does not retarget it.
The connection descriptor is a credential; do not log it or send it to a model.
Closing the app invalidates it. This recovery does not ship an external stdio
launcher: the old forwarding launchers targeted removed combined-MCP modules.
