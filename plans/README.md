# Implementation plans

Existing cold-start and Welcome plans are preserved. The CLI review plan below is grounded in `ea730cc18` and covers the two requested findings only.

| Plan | Priority | Status | Execution order |
| --- | --- | --- | --- |
| [001: CLI join review fixes](001-cli-join-review-fixes.md) | P1 / P2 | DONE | Browser ownership handoff, local-space linking, space-only status, and browser integration verified |
| [003: Both invitation types through join](003-cli-join-both-invite-types.md) | P1 | DONE | Commits `01da82175`..`d755e9c5f`; native gates and local-service integration passed; live browser and external hosted checks not run |

No additional findings were audited or rejected. Earlier CLI connection plans removed from this branch are historical context, not active prerequisites.
