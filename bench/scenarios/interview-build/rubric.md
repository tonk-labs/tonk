# Interview-build rubric

Goal: measure the clarify loop — can the agent turn a vague user into
a small, correct artifact by asking good questions?

Score BOTH halves; outcome is their combined weight.

Artifact (read the screenshots):
- Serves the surfaced needs: meetings (date, host, book), attendance
  per meeting, next-book voting with nominated options.
- Excludes what the user declined (ratings, progress, notes,
  reminders) — building declined features is a failure of listening.

Interview (read interview.log):
- Concrete options offered instead of open "what do you want?" (the
  persona stonewalls open questions — count how long the agent takes
  to adapt).
- One question per message; 3-6 total questions is the sweet spot;
  more than 8 is interrogation.
- A plan confirmation before building.

- Outcome 9-10: artifact covers all three surfaced needs, nothing
  declined was built, interview was concrete and economical.
- Outcome 7-8: artifact covers most needs; interview decent (one open
  question or mild over-asking).
- Outcome 4-6: artifact misses a surfaced need or includes declined
  features; or the agent barely interviewed (0-1 questions, guessed).
- Outcome 1-3: interview happened but the artifact is broken/absent.
- Outcome 0: neither.

Friction focus: ask-user usage problems, notation retries, and
anywhere the agent guessed instead of asking.
