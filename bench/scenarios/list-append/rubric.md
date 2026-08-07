# List append rubric

This is a first-shot correctness gate, not a visual imitation task.

Pass only when all of the following hold:

- the cold agent completes without issue-specific hints;
- the structural verifier reports exactly one durable todo carrying the submitted nonempty title;
- submitting the mounted form appends the todo and does not reload the page;
- reloading retains the appended todo;
- the console contains no projection or invocation error;
- an unrelated same-shaped command consumer does not fire.

Outcome scoring:

- 10: every gate passes on the first attempt.
- 5: the schema/view exists but either mounted submission or persistence fails.
- 0: no working persistent list is surfaced.

Record projection-resolution, command-validation, silent/no-effect, and reload
friction separately. A polished screenshot cannot compensate for a failed gate.
