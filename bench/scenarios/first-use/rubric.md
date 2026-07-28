# First-use rubric

Goal: measure how quickly a capable agent with no Tonk-specific instructions
turns its first CLI probe into a correct live-state read and one precise write.

Outcome:

- 10: “Draft launch email” is done; “Book venue” remains not done; there are
  exactly two tasks.
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
