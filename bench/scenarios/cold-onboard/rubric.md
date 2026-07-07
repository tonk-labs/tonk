# Cold-onboard rubric

Goal: measure the cold-start journey — an agent handed nothing but the
pasted invite prompt, with the tonk CLI not installed. Mechanics
(install → join → orient → push) are the point; the built artifact
matters less than in other scenarios.

- Outcome 9-10: agent got tonk running (npx or equivalent), joined the
  repo, oriented (schema/concepts/guide before writing), and pushed at
  least one coherent renderable addition (concept + data + view)
  visible at the `space` checkpoint.
- Outcome 7-8: joined and pushed real data, but the addition is weak
  (no view, or view broken) or orientation was skipped.
- Outcome 4-6: joined successfully but pushed nothing useful.
- Outcome 1-3: got the CLI running but never completed the join.
- Outcome 0: never got a working tonk command executed.

Friction focus: everything before the first successful `tonk join` —
install discovery (the prompt does not currently mention npx or where
to get the binary), sandbox/path fights (the prompt hardcodes
`mkdir -p ~/tonk/...`, which the episode sandbox denies), and
orientation toil after joining. Quote the exact prompt line that
misled the agent when you can.
