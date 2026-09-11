# Preserve Welcome across seed checks

2026-09-10. Regression after rebasing onto the seed-provenance changes in #926.

## Failure

Fresh Welcome creation evaluates core plus the repository name, evaluates an
agent supplement, and imports the Welcome application snapshot. The seed helper
started recording each of those evaluated inputs as a seed from
`/library/core.yaml`. The mount-time upgrader therefore found a digest different
from core and reapplied core over the snapshot. Its cardinality-one
`id:tonk/space` name changed from `tonk:vault/workspace` to `tonk:blank`.

An isolated browser reproduced the reported agent-sign-in canvas. Read-only
queries confirmed that Welcome content remained present while the home alias
pointed at `tonk:blank`. Changing only that alias in the disposable profile
restored the full Welcome page. The log showed the upgrade withdrawing 15
agent-library claims and evaluating core immediately before the failure.

## Correction

The onboarding-only library evaluation helper no longer records composite inputs
as a replaceable core seed. Ordinary space creation retains its existing seed
provenance recording. Mount-time automatic upgrades recognize the durable Welcome
snapshot marker and leave the imported application's authored state alone,
including snapshots created with the erroneous seed records. This intentionally
avoids replaying a library or the snapshot over user changes.

This prevents recurrence; it does not automatically rewrite the home alias of
already affected spaces. A deliberate blank/custom home cannot safely be inferred
to be accidental from its current value alone. For the reported local space,
recovery is a one-fact assertion of `id:tonk/space -> tonk:vault/workspace`, guarded
by checks that the current alias is blank and Welcome content still exists.
Automatic approval review requires explicit user approval for that personal-data
repair; no repair was applied to the user's browser during this work.

## Validation

The native regression failed before the fix with the same core replay seen in
the browser. After the fix, all 148 `tonk-worker` library tests passed, including
fresh Welcome mount checks and a legacy-record fixture preserving a custom home.
`cargo fmt --all` and `git diff --check` passed. Browser validation of the rebuilt
artifact is recorded below when complete.

A separate Trunk build succeeded (`029d50468b6f293f`, worker Wasm
`33321540e44404be`). Isolated Chrome rendered the full Welcome page on a fresh
origin. A read-only query returned `tonk:vault/workspace` for the home alias,
and the worker log contained no seed replay. The saved Welcome URL also rendered
its heading after reload. The temporary server and isolated browser were stopped.
The user's running artifact and stored space were not replaced.
