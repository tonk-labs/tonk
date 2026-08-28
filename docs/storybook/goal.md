# Storybook goal

Maintain a complete, outside-in map of Tonk user behavior entered through the
browser shell or CLI, and turn every unproved hot path into an executable,
failure-aware coverage target.

## Standing rules

1. Start from the [state model](foundations/state-model.md), not from a crate.
   A flow must name its starting state and durable ending state.
2. Use stable journey IDs from the [catalog](journey-catalog.md). New tests,
   triage entries, and verification rows should cite them.
3. Do not call a journey covered until its normal path, rejected path,
   interrupted path, durable postcondition, and safe retry are all evidenced.
4. Prefer one invariant-focused lower-layer test plus one whole-journey test.
   Avoid duplicating an expensive browser journey when a contract test can
   exhaustively enumerate response variants.
5. Account tests must keep profile DID, root DID, account provider, attachment,
   customer status, account-repository status, and space ownership separate.
6. Destructive tests must prove both the intended deletion and the boundaries:
   joined spaces, unrelated profiles, unrelated accounts, local replicas, and
   already-revoked devices remain in their documented state.
7. Every asynchronous boundary gets cancel, restart, timeout, duplicate-submit,
   response-lost-after-commit, and concurrent-actor consideration.
8. Test failures are evidence. Record the exact symptom and current hypothesis
   before changing production behavior.
9. Keep the product source read-only while doing a description pass. Create a
   separate implementation plan before adding or changing tests and code.
10. Pin every audit and verification pass to a commit and name anything that was
    not actually executed.
11. Treat a user-visible product change or bug fix as incomplete until its
    screen, journey, verification, or triage IDs are updated, or the pull
    request records why the visible contract is unchanged.
12. Keep captures honest: running product, production-source fixture, and
    captured CLI output are distinct evidence classes and must remain visible.

## Completion condition

The storybook is complete when:

- every CLI command and browser route maps to at least one journey ID;
- every journey declares all meaningful state variants and error boundaries;
- every P1 and P2 verification item either passes or has a triage entry;
- every P0/P1 automated gap in the coverage model has an executable test;
- account and destructive journeys pass in a fresh browser, a returning
  browser, the CLI, and the hybrid browser/CLI environment;
- restart and concurrency coverage proves the durable invariants rather than
  only the final text output; and
- the running-product pass agrees with the written behavior or the difference
  has an explicit product decision.

Source audit pinned to Tonk commit `a3f8670b1`. Visual inventory pinned to Tonk
commit `49a873a23`; its screenshots do not constitute a complete journey
verification pass.
