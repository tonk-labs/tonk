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

Friction focus: everything before the first successful join — whether
`npx @tonk/cli` resolved and ran cleanly (the prompt now leads with it,
so no global install is needed), whether the agent handled the
"ask me where to keep the tonk" step sensibly instead of stalling or
picking a denied path, and orientation toil after joining. Quote the
exact prompt line that misled the agent when you can.
