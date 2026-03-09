# Agent Orchestration — Notes & Opinions

Running notes on how I think about and structure LLM-assisted work.
Updated as I learn what actually works vs what sounds good in theory.

---

## Mental Model

Think of the agent as a very fast, sometimes-overconfident junior engineer.
It needs the same things a junior needs:
- Clear scope
- Relevant context (no more, no less)
- Explicit success criteria
- A way to ask questions before making assumptions

The failure mode isn't usually "the LLM is dumb". It's "I gave it a vague task and a 40k-token context and hoped for the best".

---

## Context Is the Job

The single highest-leverage thing you control is what's in the context window.

**Principles:**
- Load what's needed, nothing else. Every irrelevant token crowds out relevant ones.
- Order matters: most important context first, least important last (recency bias is real).
- If you're repeatedly having to remind the agent of something, it should be in CLAUDE.md, not in every prompt.
- Large `<paste entire codebase>` approaches are almost always worse than targeted, queryable context.

**What I do:**
- Keep CLAUDE.md tight and up to date — the agent loads this in every session.
- Per-project `docs/architecture.md` for the 5–10 most important decisions in that repo.
- Reference external docs by URL or via Context7 (`use context7`) rather than pasting them in.
- When starting a complex task, I explicitly tell the agent which files are relevant.

---

## Task Scoping

The narrower the task, the better the output. This is always true.

**Good:** "Refactor the `parseToken` function in `lib/auth/tokens.ts` to use the `Result` type instead of throwing. Add a test for the error path."

**Bad:** "Improve the auth system."

If a task is too big to express in a sentence, it's too big for one agent turn.
Break it down first. It takes 5 minutes and saves 30.

---

## When to Use Subagents

Subagents (parallel agents, task delegation) are useful when:
- Tasks are genuinely independent and can run in parallel
- You want to protect the main context from exploratory research noise
- A task is repetitive across many files (e.g., "check each of these 20 modules for X pattern")

Subagents are **not** useful when:
- Tasks depend on each other's output (sequencing, not parallelism)
- The task is simple — spinning up an agent adds latency and overhead
- You need tight feedback loops

I use subagents for research (find things) and keep the main session for writing (change things).

---

## Prompting Patterns That Work

### "Read before you write"
Don't ask the agent to modify a file it hasn't read.
Always front-load: "Read `foo.ts` first, then..."

### "Plan, then execute"
For anything touching more than 2–3 files, ask for a plan before execution.
Review the plan. It's much cheaper to correct a plan than to roll back 8 file changes.

### "Show your reasoning"
When I'm unsure what the agent is going to do, I ask it to explain its approach before coding.
"What changes will you make and why?" catches misunderstandings before they hit the file system.

### "Explicit success criteria"
"Add pagination to the user list endpoint. Done means: the endpoint accepts `page` and `pageSize` query params, returns `{ data, total, page, pageSize }`, and has tests for boundary conditions."

### "Negative space"
Telling the agent what NOT to do is as important as what to do.
"Refactor `parseUser` — but don't change the return type or add new dependencies."

---

## Failure Modes I've Hit

**The silent workaround:** Agent can't do X, so it quietly does Y instead without saying so.
*Fix:* Explicitly ask "if you can't do X, stop and tell me rather than finding an alternative."

**The over-architecter:** Given a simple task, agent introduces a whole new abstraction layer.
*Fix:* "Solve this with minimal changes. No new abstractions unless you explain why first."

**Context bleed:** The agent carries assumptions from an earlier part of the session into a new task.
*Fix:* For major context shifts, start a new session. Or explicitly say "forget what we were doing before."

**The hallucinated API:** Agent uses a function that doesn't exist or a method signature that's wrong.
*Fix:* Have the agent read the actual library source or docs first. Don't trust its training data for fast-moving APIs.

**The infinite refactor:** "Improve the code" leads to changes everywhere.
*Fix:* Never use vague improvement prompts. Always specify scope.

---

## On Agentic Loops & Long-Running Tasks

I'm skeptical of long autonomous loops without checkpoints.

My preference: human-in-the-loop at each meaningful step.
The agent does a step, shows me the output, I approve, then it continues.

For truly automated workflows (CI bots, cron agents), I require:
- A well-defined stopping condition
- A list of what the agent is allowed to do (no open-ended permissions)
- Logging of every action taken
- A human review gate before anything hits production

*Related reading:* Anthropic's approach to computer use tasks — the principle of "minimal footprint" (do the least needed to accomplish the task) is right.

---

## Context Management as Debt

Every "just paste the whole file" decision is context debt.
Eventually the context fills up, quality degrades, and you have to start over.

I think of context the same way I think of technical debt: manageable if you're intentional, ruinous if you're not.

**Good habits:**
- Summarize long explorations before continuing ("what we've established so far is...")
- Remove completed sub-tasks from the active context
- Don't re-explain things the agent already knows — trust the context, or start fresh if you can't

---

## Outstanding Questions (for me to keep thinking about)

- How do I version my agent configs (CLAUDE.md etc.) across projects without drift?
  → Probably a private shared monorepo with a bootstrap script
- How do I share a set of agent configs across platforms (Claude, Cursor, Copilot)?
  → Platform-agnostic format? The `system_prompt` abstraction differs everywhere.
- When does a queryable knowledge base (like Carry) beat a flat context dump?
  → Clearly better for long-lived reference material (style guides, architecture docs)
  → Less clear for frequently-changing project state
- Agent memory vs. manual CLAUDE.md maintenance — what's the right split?
