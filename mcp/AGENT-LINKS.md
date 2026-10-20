# Agent-link MCP

`tonk_query` and `tonk_evaluate` require `{ agent_link, document }` on every call.
Copy the complete agent link from the desired Tonk space, including its private
fragment. The hosted `/mcp` endpoint no longer uses account OAuth. Existing
Desktop signed-in tools and the optional account adapter retain their contracts.

## Local coding agents

```sh
cargo build --locked -p tonk-worker --features native-host --bin tonk-agent-worker
npm ci --ignore-scripts --prefix mcp
```

Configure a stdio MCP server with command `node` and arguments:

```text
/absolute/path/to/mcp/scripts/agent-link.mjs BINARY DATA_DIRECTORY TRUSTED_REMOTE LINK_ORIGIN
```

Use an absolute native binary path and a dedicated private data directory (0700).
`TRUSTED_REMOTE` must exactly match the signed access-service URL, including its
path and trailing slash. `LINK_ORIGIN` pins the origin serving copied shortcuts;
it defaults to the remote's origin. These are trusted host configuration.

The adapter resolves at most one same-origin shortcut redirect, without sending
the private fragment to the shortcut server. Expanded links and notation travel
to the native process on stdin, never command-line arguments. Raw native logs
are discarded. The link remains a bearer credential visible to the agent platform;
do not put it in shell history, diagnostics, or committed configuration.

## Hosted clients

Use the existing deployed `/mcp` Streamable HTTP URL with no OAuth configuration.
Discovery is public. Every invocation validates the link's signature, recipient
key, exact scope set, expiry and pinned remote, then obtains fresh remote access.
Missing links or revoked read authority never fall back to an account or cached data.
The signed-in Desktop may continue using its existing internal worker; external
agents receive only the space authority represented by their link.

The hosted executable uses `TONK_MCP_ISSUER`, `TONK_PROVIDER_ORIGIN` and
`TONK_AGENT_WORKER_BINARY`. Its closed replica directory is restored from and
saved to a separate conditional R2 checkpoint (`agent-state-v1.json`) in the
`agent-link-v1` container namespace. Old OAuth checkpoints are retained, unused.
No migration imports account authority into agent replicas. Old OAuth clients
must reconnect without OAuth and supply a link on each call. Embedded Apps UI
and account listing are not exposed by this endpoint.

## Execution and recovery

Replicas are isolated by subject, recipient and signed grant set. Native processes
hold an exclusive root lock. The adapter admits at most 32 queued/running calls;
each process has a two-minute deadline and a three-second forced-exit grace.
Documents are limited to 32,000 UTF-8 bytes. Storage admission is capped at 64
replicas, 192 MiB and 3500 files; capacity failure requires operator attention.
There is no automatic eviction of retained work. These are bounded deployment
limits, not measured capacity targets.

Client cancellation stops queued calls and active queries. Once an evaluation
is dispatched, it finishes under the same two-minute process deadline and saves
its checkpoint even if the client disconnects. Graceful hosted shutdown drains
the active call and rejects queued calls. A disconnected client must still query
before deciding what to do next; completion does not guarantee delivery of its
response, and a platform kill can still interrupt the call.

Normally the worker pulls, evaluates once, and pushes writes. Before evaluation
it synchronously persists a pending marker. If evaluation, push, or process
termination leaves an unknown outcome, subsequent writes are blocked and pulls
cannot overwrite retained state. Authorized queries inspect the retained state
and include `recovery.required`. They must still pass a fresh remote read check.
A checkpoint is saved after the child closes, including failure paths, before
returning the outcome. Checkpoint failure freezes subsequent calls in that host.

Recovery is an explicit operator operation, not an MCP tool or automatic retry.
Stop the owning host, preserve its directory, inspect retained state with a query,
then invoke the native binary against that same directory and trusted remote.
Supply this JSON on stdin through a secret-safe mechanism:

```json
{"tool":"publish_retained","agent_link":"COMPLETE_EXPANDED_AGENT_LINK","document":""}
```

This publishes retained state without repeating notation. It requires a currently
valid link for that same grant set; a revoked/expired grant cannot authorize
recovery. Do not delete unresolved directories to make a retry succeed. For hosted
recovery, restore the private checkpoint, recover offline under operator control,
and conditionally persist it before restarting the service. Never edit a live
checkpoint concurrently with its container.

Delivery is at most once per invocation, not distributed exactly-once execution.
Container loss before the post-call checkpoint can lose unpublished local state;
remote effects may already exist. A lost response never proves rollback. Inspect
the remote space before deciding further action. Do not automatically repeat an
uncertain evaluation. Durable acknowledged state does not imply durable in-flight
state. The service does not deduplicate separate client submissions.

## Verification

```sh
npm test --prefix mcp
npm run test:hosted --prefix mcp
cargo test --locked -p tonk-worker --features native-host --lib agent_link
cargo test --locked -p tonk-worker --features native-host --test agent_link
```

The integration fixture starts the real local access service and S3 emulator,
exercises stdio and HTTP through the native binary, independently reads remote
writes, and tests retained-state recovery and remote revocation. It requires
Node, installed MCP dependencies and loopback listeners. These checks do not
establish Docker/Cloudflare deployment or real hosted-client acceptance.
