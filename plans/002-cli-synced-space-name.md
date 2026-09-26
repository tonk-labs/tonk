# Synced CLI space names

The scoped invitation importer currently retains `agent-<connection-id>` as its
local alias even after pulling the repository name. Reuse the existing synced
name selection and keep the connection receipt protocol unchanged.

- Replace the temporary alias after successful synchronization, atomically
  updating every directory binding without moving replica data.
- Preserve explicit aliases and choose a numeric suffix for collisions.
- Apply the same correction when resuming an existing generated alias.
- Verify naming, collisions, replay, explicit aliases, and receipt behavior.

Ordinary invite URLs are currently rejected by the CLI command. Unifying those
authority/import protocols is outside this naming correction.

Implemented and verified with `cargo test -p tonk-cli --test handoff --test
connection_commands --locked`: 16 tests passed. The local service fixture required
execution outside the sandbox after an `Operation not permitted` failure.
`cargo fmt --all` completed. Browser toast display and hosted sync were not tested;
the existing receipt publication and rendering tests passed.
