# Tonk MCP core

Tonk's general agent API is `tonk_query` and `tonk_evaluate`. Both send inline
notation unchanged to the ordinary worker. Hosts supply authority and transport;
the core has no ChatGPT UI, hosted login service or Desktop dependency.
The hosted service has two endpoints. `/mcp` is an OAuth endpoint where a person
sponsors an agent connection: the agent gets its own persistent identity, and the
person's device grants it `/provider/add` on their account (see
`hosted/sponsor-service.mjs` and `plans/2026-10-28-hosted-agent-accounts.md`).
`/mcp/link` takes [per-call agent links](AGENT-LINKS.md), with no OAuth. The same
agent-link backend supports local stdio coding agents.

## Source map

| Files | Responsibility |
| --- | --- |
| `primitive.mjs`, `errors.mjs` | Portable query/evaluate dispatch and read-only gate |
| `tools.json`, `catalog.mjs` | Shared schemas, descriptions and target selection |
| `core.mjs`, `http.mjs` | MCP registration, results and generic HTTP transport |
| `browser.mjs` | Same tools and guides for a host using Web APIs without the MCP SDK |
| `account-backend.mjs`, `account-spaces.mjs` | Optional account discovery and per-call authorized selection |
| `authoring.mjs`, `authoring-resources.mjs` | Shared context and canonical CLI manuals |
| `worker-process.mjs`, `integration/` | Private native worker process and integration verification |

Generated `authoring-content.mjs` comes from the canonical CLI manuals, including
`rust/tonk-cli/src/guide-agent.md` for shared agent instructions. Edit their
sources, then run `npm run build:guides`; `npm run check:generated` detects drift.
There are no restricted-runtime or feature-specific compatibility tools.

## Host integration

```js
import { createAgentTools } from './browser.mjs';
import { createTonkHTTP } from './http.mjs';

const http = createTonkHTTP(async (requestInfo) => {
  // Implemented by your host: authenticate this request and select its authority.
  const connection = await authorize(requestInfo);
  const agent = createAgentTools({
    subject: connection.subject,
    allowsWrites: connection.allowsWrites,
    request: (path, options) => connection.worker.request(path, options),
  });
  return { capabilities: agent.tools.map((tool) => tool.name), call: agent.call };
});
// Adapt http.fetch(Request) and http.close() to your host's HTTP listener.
```

For other MCP transports, construct a server with `createTonkServer(backend)`
from `core.mjs` and connect the SDK transport. A browser host can call
`createAgentTools` directly. A primitive imports no Node, MCP SDK or UI code.

An account resolver must authenticate before returning a backend. An agent-link
backend instead validates the supplied scoped credential on every tool call. Do not publish a
personal worker as a singleton backend on a public listener. For multi-space
accounts, `accountBackend({ principal, tenants, allowsWrites })` adds
`tonk_list_spaces` and explicit `space` arguments. The host implements
`tenants.list(principal)` and `tenants.resolve(principal, subject)`; resolve must
revalidate membership every call and return only that principal's authorized
worker. `accountSpaces(worker)` reads the ordinary worker's account catalog.
The core does not mint credentials or manage account lifetime.

## Contract

`tonk_query` calls the selected space's main-branch evaluator with
`transact=false`; `tonk_evaluate` uses `transact=true`. Results preserve the worker
JSON envelope. Inputs cannot override the space, branch, origin or local paths.
Read-only hosts omit evaluation and reject direct attempts to call it. Documents
are limited to 32,000 UTF-8 bytes; result JSON is limited to 2,000,000 bytes.

Dispatch is at most once per invocation, not distributed exactly-once delivery.
If a reply is lost, a write may already have committed. Query first; never replay
automatically. Synchronization and rendering are separate host observations.
The host must honor the supplied abort signal; cancelling a dispatched write
does not prove rollback. Remote account mutation and UI confirmation are outside
this core's authority.

Query `concept:` to discover schemas and `tonk/repository:` for space metadata.
Use the spaces guide's command recipe for rename and the ordinary evaluator for
library installation. `tonk_guide` and `tonk://guide/<topic>` serve the same manuals.
Account discovery is optional; account-level creation remains a separate host
capability. There is no arbitrary account/profile evaluator.

## Development

Requires Node 22 or later. From `mcp/`:

```sh
npm ci --ignore-scripts
npm test
npm run check:format
```

Run the ordinary-worker integration from the repository root:

```sh
cargo build --locked -p tonk-worker --features native-host --bin tonk-worker-host
TONK_WORKER_BINARY="$PWD/target/debug/tonk-worker-host" npm --prefix mcp run test:integration
cargo test --locked -p tonk-worker --lib native_host_tests
```

Integration tests fail if `TONK_WORKER_BINARY` is missing. The worker host binds
only loopback, requires a private bearer token, and opens an explicit data
directory; it is not a public MCP listener. Tests use disposable local data and
do not establish live account linking, remote persistence or frontend rendering.
The MCP workflow runs formatting, generated-guide checks, core contracts, worker
library tests (including native storage/restart) and the generic-client/native-worker
integration independently of any application integration.
