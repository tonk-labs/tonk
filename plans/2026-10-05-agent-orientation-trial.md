# Agent orientation trial — 2026-10-05

## Setup

- Fresh subagent created with `fork_turns="none"`; no prior conversation supplied. It was instructed not to consult memory, repository files, or external documentation.
- Current working-tree CLI built with `cargo build -p tonk-cli --bin tonk`.
- Disposable local space, separate profile and registry, no upstream; telemetry and update checks disabled.
- Task: mark “Draft launch email” done and change nothing else.
- Fixture: two tasks, one note, one contact, and a short shared instruction to preserve unrelated records.
- Agent received the updated UI handoff paragraph after connection, verbatim except for substituting the isolated executable path. Connection was already complete.
- CLI wrapper recorded arguments, timestamps, exit codes, stdout, and stderr; it did not change command output.

## Observed commands

Times are seconds since the first CLI invocation; setup, build, and agent startup are excluded.

| Start | Command | Exit |
| ---: | --- | ---: |
| 0.00 | `tonk --space bench space agents get` | 0 |
| 2.39 | `tonk --space bench show` | 0 |
| 6.59 | `tonk --space bench query task` | 0 |
| 6.68 | `tonk --help` | 0 |
| 9.12 | `tonk help assert` | 0 |
| 13.77 | `tonk --space bench assert task <ENTITY> --done true` | 0 |
| 13.87 | `tonk --space bench show task <ENTITY>` | 0 |

## Result

- Verification finished 13.90 seconds after the first CLI invocation.
- Seven commands, one data query, one write, no command errors.
- Independent before/after comparison confirmed the only data change was the requested task’s `done` field from false to true; entity IDs and the other task, note, and contact were unchanged.
- Full schema output was 55383 bytes / 1434 lines. The agent reported tool-output truncation but could see the task schema.
- Agent used two targeted help calls before writing; it did not query unrelated concepts or enumerate views.

## Limits

One small local trial with the new handoff instructions, not an A/B comparison or a production timing measurement. It does not test invitation setup, remote latency, large real-world spaces, or CLI-only discovery without the handoff prompt. The subagent did not inherit conversation history; standard agent instructions still apply.

Raw command logs and baseline/final snapshots: `/private/tmp/tonk-agent-orientation-20261005/`.

## Compact overview follow-up

Implemented bare `tonk show` as a typed application-concept overview, limited to
20 concepts and eight fields per concept. Optional fields use `?`, many fields
use `[]`; descriptions are single-line and limited to 80 characters. `--all`
includes runtime concepts and expands the concept/field limits. Named inspection
remains available. The existing JSON contract is unchanged; `--notation` is the
full export and matched the earlier 1,434-line output byte for byte.

The same fixture now produces 16 lines / 433 bytes (previously 1,434 lines /
55,383 bytes). It reads schema metadata only, without instance counts or samples.

A second fresh subagent received the same task and handoff text, with only the
isolated executable path changed. Its executable was frozen from the fresh
Cargo integration-test build, and its space started from the same fixture.

| Start | Command | Exit |
| ---: | --- | ---: |
| 0.00 | `tonk --space bench space agents get` | 0 |
| 4.39 | `tonk --space bench show` | 0 |
| 6.98 | `tonk --space bench query task` | 0 |
| 10.58 | `tonk --space bench assert task <ENTITY> --done true` | 0 |
| 13.54 | `tonk --space bench show task <ENTITY>` | 0 |

Verification finished 13.58 seconds after the first CLI call.
Five commands, one data query, one write, no help calls, no errors. Independent
before/after comparison confirmed only the requested `done` field changed.
The other task, note, contact, and all entity IDs were unchanged.

This demonstrates smaller output and two fewer commands in this run. The
13.58-second completion time versus 13.90 seconds is not evidence of a speedup;
one run per version cannot establish an agent-performance improvement.

Validation: all five focused tests passed: three renderer tests, the CLI parser
check, and the CLI dispatch/export/JSON integration check. Formatting and
whitespace checks passed. The CLI test waited for another active Cargo build
before completing successfully.

Raw second-trial evidence: `/private/tmp/tonk-agent-compact-20261005/`.

Observed overview:

```text
Space: bench

Concepts (3)
  contact  name: text
  note  body: text
  task  done: boolean, title: text

? = optional; [] = many

Inspect instances:  tonk query <concept>
Inspect schema:     tonk show <concept>
Update an instance: tonk assert <concept> <entity> --<field> <value>
Verify:             tonk show <concept> <entity>

Runtime concepts omitted; use tonk show --all to include them.
Full schema export: tonk show --notation
```
