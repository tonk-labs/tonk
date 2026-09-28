# Agent link bootstrap prototype

Status: prototype implemented; local checks passed; deployment acceptance pending.

Goal: a scoped tool link alone lets a shell-capable agent discover how to join
and orient in the space. Preserve the existing grant format, fragment secrecy,
person/tool distinction, deployment verification, and confirmation receipt.

Implementation: standalone `/agent/` static HTML copied by Trunk; new worker
tool invitations target it; existing root and `/join` shells advertise it through a head `rel="help"` link; successful
CLI joins print the existing space instruction and schema inspection commands.
HTTP GET remains passive and the document uses no scripts or external assets.
Existing playground-specific prompt constraints remain unchanged.

Verification: static document HTTP/discovery test, build manifest inclusion,
CLI direct and shortened invitation routing, connection command integration,
Rust formatting. Browser/Wasm packaging and a fresh agent using only a fixture
link are separate acceptance gates; do not claim those from static checks.

Deferred: structured join output, per-invitation task context, automatic surfacing
of space instructions, and making copy-link replace copy-prompt in onboarding.

## Validation (2026-09-25)

- `cargo fmt --all -- --check` and `git diff --check`: passed.
- `cargo test -p tonk-cli --test join_routing --test connection_commands`: 9
  passed with loopback access. The initial sandboxed integration run failed
  starting a fixture service (`Operation not permitted`).
- Build-artifact, boot-script and service-worker tests: 95 passed together;
  after adding development routing, both focused agent-route tests passed.
  Node's `--test-force-exit` was used because worker timers retain the process.
- Isolated headless Chrome loaded the fixture URL from a local static server:
  instructions present in the accessibility tree, zero document scripts, no
  horizontal overflow. No connection/API traffic; only document and favicon.
- A new regression test first exposed that the worker always served the SPA
  despite its existing static-document comment. The prototype now routes only
  the explicit agent documents through the retained asset cache (or development
  server), preserving ordinary app navigation.

Not run: full Trunk/Wasm build, packaged browser connection journey, Cloudflare
routing/CORS verification, or an independent fresh-agent acceptance run. No
production invite redeemed or deployment performed.

## Local testing follow-up

- Removed the agent discovery sentence from the visible boot shell; the HTML
  head now advertises the instructions without adding loading-screen content.
- Added a direct copy-link action to the attached agent panel, retaining the
  optional copy-prompt action. Browser coverage verifies the exact clipboard
  payload, including the fragment.
- Account completion uses an explicit agent resume event which clears a stale
  account refusal without duplicating an in-flight or ready invitation.
- Contained account return closes in place instead of navigating away before
  its deferred completion callback can reach the guest.
- Agent-panel Chromium tests: 7 passed. Contained-return Chromium regression:
  1 passed. Boot and build-artifact tests: 22 passed. Rust formatting and
  whitespace checks passed. Full real-account signup through the packaged app
  has not been rerun.

## E2E CI repair (2026-09-28)

Run 36400637578 at f2602b503 failed 12 tests across three shards:

- Ten offline-generation-dependent tests could not finish cache adoption.
  Caddy's `try_files {path} /index.html` served the SPA at `/agent/`, whose
  manifest entry requires the static agent document. Add the directory index
  candidate before the SPA fallback. A local HTTP probe reproduced the old
  response and verified the corrected document hash, while `/space/example`
  still receives the SPA.
- Two connection tests used `.panel-copy`, which now selects the first of two
  buttons (copy link). Select `.agent-copy-prompt` and `.agent-copy-link`
  explicitly, exercising both and preserving the existing prompt checks.

Current Nix preview, native E2E test binary, and CLI builds passed. Rust and
Nix formatting and whitespace checks passed. On Chrome 154, the 12 previously
failing tests ran serially without retries: 11 passed; the busy-page successor
test reached complete generation adoption but timed out with the successor
installed and waiting. This is a later boundary than the CI asset-hash error.
The isolated busy-page test passed on Chrome/ChromeDriver 150.0.7871.115
(CI uses 150.0.7871.114), in 124 seconds. The Chrome 154 failure remains a
validation caveat: one comparison does not prove the browser version caused it.
The 11-test success plus this focused pass covers all originally failing test
names, but is not a clean single-browser suite run. A fresh hosted run remains
pending.

Local evidence: `/tmp/pr1012-e2e.log`, `/tmp/pr1012-chrome150.log`, and
`/tmp/pr1012-artifacts.log`. The browser artifact was built from f2602b503
with `connection-invites`; only the Caddy fixture and native test helpers were
changed for this repair. Test environments used isolated profiles and localhost;
CI uses tonk.network with its loopback mapping.
