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
