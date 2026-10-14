# CLI agent workflow improvements

Scope: build on the current uncommitted compact `show` and handoff changes without
altering unrelated work. No commits or publication requested.

1. Typed exact-equality query filters: repeated `--where FIELD=VALUE` constraints
   combined with AND. First slice supports text and boolean fields, rejects
   unsupported types and duplicate fields, and evaluates constraints in Dialog.
   Existing unfiltered output remains compatible. No silent truncation or
   pagination in this slice; many-valued filters match individual values.
2. Explicit assert receipts: report local commit and requested-field read-back
   separately; opt-in JSON, unchanged schema field precedence. Dry runs must
   never report applied/verified. Read-back failure must retain the commit
   receipt and recovery guidance. Sync warnings must not imply remote success.
3. Missing shared instructions: successful empty result, while read errors fail.
4. Restore stored concept descriptions without dropping undescribed concepts.
5. Refresh the first-use benchmark commands and exercise larger/ambiguous data,
   missing instructions, and sync-warning behavior through focused local tests.

Validation: narrow tests for each slice, then relevant CLI/parser/data/schema
suites, `cargo fmt --all -- --check`, `git diff --check`, shell syntax checks,
and a fresh isolated agent trial. Preserve raw trial data and compare final
state independently. Do not infer timing improvements from single trials.

Status:
- Typed text/boolean AND filters implemented; first two focused tests passed.
- Structured assert receipts implemented; creation, update, dry-run and repeated
  write check passed. The test initially assumed repeat assertions report zero
  claims; actual Dialog behavior reports a claim, so the test now checks stable
  entity identity and field values instead.
- Missing instructions empty-success check passed.
- Batched description recovery implemented. Corrected the description test to
  create a genuinely undescribed concept; `concept add` supplies a default.
  Updated the older test that deliberately pinned the former fidelity gap.
- First-use scripts refreshed, historical/current transcript metrics pass.
- Regression tests caught a real receipt error: two read expressions are
  independent, so entity existence alone did not prove the requested values.
  Verification now requires both projections to match. Wrong-value and missing
  entity tests cover this; both pass in the final rebuilt suite.
- Updated the remote failure fixture to wire an upstream explicitly (the library
  registration API does not do the CLI command's extra setup), and refreshed the
  assert-help expectation for typed filters.
- Initial broad sandbox test run stalled on an account localhost callback. The
  exact test passed with host loopback access in 0.43 seconds; tests now run with
  that access. The first completed pass exposed the failures above; it was not
  counted as passing.
- Script hooks passed for 2 tasks and 253 tasks including a duplicate title and
  shared instructions. Independent baseline comparisons prove only the intended
  done field changed. Artifacts: `/private/tmp/tonk-benchmark-workflow-{small,stress}-20261005`.
  The stress harness initially omitted `space.did`; repaired harness metadata
  and completed its claim/projection checks before running scripted/verify.
  This checks native scripts, not the browser/remote benchmark runner.

Initial fresh trial (before the final receipt regression fix):
- No conversation history, memory or source access; only a task, isolated CLI and
  the product handoff. 252 tasks, one note and one contact; duplicate task title,
  no shared instructions, no upstream.
- Four calls: `space agents get`, `show`, filtered task query, assert on the
  unfinished match's canonical entity. 11.88 seconds from first CLI invocation
  through final completion; no help calls or separate verification read.
- Independent comparison confirmed exactly the requested false-to-true change;
  all other tasks, notes, contacts and IDs unchanged. This is one observation,
  not evidence of a timing improvement over earlier fixtures.
- Raw evidence: `/private/tmp/tonk-agent-workflow-20261005`, including
  `agent-commands.jsonl`, `baseline.json`, `after.json`, handoff and frozen binary.

Final validation after the last source change:
- `cargo test -p tonk-cli --lib --bin tonk --test data_verbs --test schema_read
  --test cli_space --test sync --test site -- --test-threads=1`: 457 passed,
  zero failures. Host loopback access used for local callback/sync fixtures.
- `cargo test -p tonk-fab --lib tool_connection_query_and_prompt_preserve_the_scoped_link`:
  one passed. Total 458 relevant Rust tests passed; no full workspace or rendered
  browser test claimed.
- `cargo fmt --all -- --check`, `git diff --check`, shell syntax, historical and
  current transcript metrics checks passed.
- Replayed the fresh agent's exact four-command trajectory on a new isolated
  fixture using the final binary (substituting the newly seeded target identity).
  Receipt reported verified/local and no-upstream. Independent full state
  comparison proved only the requested done field changed across 252 tasks,
  a note and a contact. This replay validates the final implementation; the
  autonomous timing observation above remains from the earlier binary.
- Final replay evidence: `/private/tmp/tonk-agent-workflow-final-replay-20261005`.
  Binary SHA256: `2990d269e7940e6758111cb7420ef9f0ba7fe29d7438a7b98b9d0e5931a99be3`.
  Test logs: `/private/tmp/tonk-agent-workflow-tests-final.log` and
  `/private/tmp/tonk-agent-workflow-fab.log`.
- Complete within the scoped slice: exact text/boolean filters only, assert
  receipts only, no pagination or browser/remote benchmark execution. Changes
  remain uncommitted.

## CI follow-up: worker session fixture isolation

Native debug job `111769282046` failed in
`session::tests::it_bounds_the_session_within_the_ttl` while opening its profile,
before the TTL assertion: `nothing grants ... the storage of ...`.
Native release was canceled without a test failure in its log.

The session fixture varies the profile name but shares `Directory::Temp` and its
`tonk-system` credential with other processes. Storage ownership is loaded once
when constructing storage and again when opening the profile; concurrent first
creation of that shared key can make those owners differ. The existing device
fixture already isolates its native directory for this reason. These worker
paths are unchanged from the PR base.

Smallest fix: use one native credential directory per session fixture, retaining
that same directory across the durable reopen test. Keep browser setup unchanged.
Add a deterministic fixture regression requiring independent storage owners;
prove it fails before isolation, then run the session tests and repeated parallel
cold starts after the change. No production credential-store redesign in this PR.

Before-change evidence:
- The freshly compiled original TTL test reproduced the exact CI storage-grant
  error locally. Log: `/private/tmp/tonk-pr1056-session-before.log`.
- `it_isolates_scratch_profile_storage_owners` failed because both profiles had
  the same storage-owner DID. Log: `/private/tmp/tonk-pr1056-isolation-before.log`.
- The fixture change follows `device::tests::scratch`: native credentials use
  `Directory::At(temp_dir/unique_name)`; the browser still uses `Directory::Temp`.
  The reopen test derives the same directory from its retained unique name.
- Final validation: all nine session tests passed with four test threads:
  `cargo test -p tonk-worker --lib --features integration-tests session::tests -- --test-threads=4`.
  Log: `/private/tmp/tonk-pr1056-session-after.log`.
- The original TTL test passed 48 separate process runs in three cold temporary
  directories, with 16 processes starting together per directory. Evidence:
  `/private/tmp/tonk-session-race-after-jzfjmqbo`; harness:
  `/private/tmp/tonk-pr1056-stress.py`.
- Formatting and whitespace checks passed. No full workspace rerun or claim of
  a production credential-store fix; remote CI must validate the pushed commit.
