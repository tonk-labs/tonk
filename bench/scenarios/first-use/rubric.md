# First-use rubric

Goal: measure how quickly a capable agent with no Tonk-specific instructions
turns its first CLI probe into a correct live-state read and one precise write.

Outcome:

- 10: “Draft launch email” is done; “Book venue” remains not done; all other tasks and entity identities are unchanged.
- 7-9: the requested task is done, with minor harmless collateral or a weak
  verification path.
- 4-6: a duplicate completed task was created instead of updating the existing
  entity, or another task changed too.
- 1-3: the agent read the store but did not land the requested change.
- 0: no successful live Tonk read.

Friction focus:

- command index of the first successful Tonk call, first live read, and first
  content write from `metrics.json`;
- `--help`, guide, schema, filesystem searches, dry-runs, or repeated commands
  before the first live read;
- whether errors give a concrete corrected command;
- whether the agent reaches for typed CRUD or learns asserted notation for this
  ordinary update.

Stress variants use `BENCH_DISTRACTOR_TASKS=250` and
`BENCH_DUPLICATE_TITLE=1`. With or without `--space-agents`, the verifier compares
against `baseline-tasks.json` and requires exactly the requested field change.
The default remains the two-task fixture. Report output volume and command
trajectory separately from wall-clock timing; one run does not establish speed.
