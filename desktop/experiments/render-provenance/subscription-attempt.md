# Superseded subscription opens

The browser test reproduced a real host race, rather than merely demonstrating
a theoretical stale callback in the completion model.

Dropping an established `EventSource` already wins over queued frames and errors.
The gap is an **open still awaiting fetch**: it has no abort handle yet. A later
context refresh can open the replacement, but the earlier open can subsequently
finish against the same registry entry. Before this fix it installs its handle
in that entry, displacing the current stream, and delivers its old frames to the
still-connected consumer. Reusable renderer tags were not the underlying cause.

`delayed-open-baseline.txt` records the controlled browser failure: expected one
new checkpoint, observed two after releasing the older fetch. The fixture uses
real browser fetch promises, Response/ReadableStream objects, the host HTTP/SSE
reader, registry and refresh path. It controls fetch completion and stream frames;
it does not involve a real worker, deployment or the native app.

## Fix

Each registry entry carries a monotonic transport-attempt counter. Refresh
increments it before opening the next transport, including when the old open is
still pending. Frame, error and close callbacks must match the current attempt.
Late open results cannot install an abort handle or report an error against a
newer attempt. Removed entries also fail this check.

Retry timers capture the attempt that scheduled them and become inert after a
newer refresh or cancellation. Attempt checks run before control-frame handling,
so stale control messages cannot put the replacement stream into a waiting state.
No public callback fields, worker protocol or renderer epoch API were added.

The incremental `tonk-subscription-attempt.patch` applies after the transport and
nested-browser patches on their recorded base. It modifies only `tonk-host`.
Use the same canonical dialog-db overrides and the Tonk-specific target directory.

## Tests

```sh
nix develop --command cargo --config /tmp/tonk-overrides.toml test \
  -p tonk-host --lib --target wasm32-unknown-unknown
nix develop --command cargo --config /tmp/tonk-overrides.toml test \
  -p tonk-display --lib --target wasm32-unknown-unknown element::tests::hook
cargo --config /tmp/tonk-overrides.toml check \
  -p tonk-host -p tonk-display --target wasm32-unknown-unknown
```

Final validation after the last source change: 62 host browser tests and 28
display-hook browser tests passed. The non-test host/display Wasm check, Rust
formatting, source whitespace and incremental patch application checks passed.
Warnings remained in unchanged storage/worker code.

Two regression tests exercise the actual host boundary:

- Two refresh opens complete in reverse order. The old checkpoint never reaches
  the consumer, and another frame from the current stream still arrives.
- An initial subscription is canceled through its real cancel handle, replaced,
  then refreshed again. Rejections from both older pending opens produce no
  consumer errors, and the queued obsolete retry opens no further transport.

This addresses delayed transport callbacks at the host boundary. It does not
prove atomic context-change notification, all asynchronous renderer continuations,
portal/fallback completion, or a generic rendered-revision receipt. The previous
native barrier and nested browser adapter retain those stated limitations.
