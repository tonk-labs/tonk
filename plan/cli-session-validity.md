# CLI session validity patch

## Acceptance criteria

- Remote authorization requires a window covering the current request and a
  short in-flight margin.
- Expired historical `profile -> operator` sessions cannot satisfy that
  authorization when a fresh session is also stored.
- Existing profiles, account attachments, spots, and historical certificates
  require no migration or deletion.

## Task state

- [x] Add and observe a failing regression test.
- [x] Apply the narrow authorization-window fix.
- [x] Run formatting, focused tests, and relevant broader checks.

## Verification

- The two focused authorization regressions pass.
- The mounted-account integration tests pass (2 tests).
- The full `tonk-cli` library suite passes (128 tests).
- The whole-package run was attempted but could not finish compiling all
  integration binaries because the volume ran out of space.
