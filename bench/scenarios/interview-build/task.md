You are working in a directory that is a tonk site, already connected
to its remote. The tonk CLI is on PATH (`tonk guide` for the
reference).

The user wants "something for my book club" but hasn't decided any
details. You cannot see them, but you can talk to them: run
`ask-user "<your question>"` and their reply is printed. One question
per call. They are not technical — ask about their club and what they
want to keep track of, not about schemas or tools.

Interview them, then build what they need in this tonk spot: concepts,
seed data from what they told you, and views so they can see it.
Confirm the plan with them once before building. Stop when `tonk
status` reports the branch is synced.
