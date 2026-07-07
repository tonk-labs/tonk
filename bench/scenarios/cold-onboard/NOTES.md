# cold-onboard scenario notes (harness-side; not shown to the episode)

- The episode prompt is generated per-run by prepare.sh from
  core.yaml's `id:agent-invite/prompt` view — the real product copy.
  There is deliberately no task.md. To improve this scenario's scores,
  change the product (core.yaml copy, CLI behavior), not the scenario.
- The baseline copy assumes `tonk` is installed; episodes are expected
  to flail here at first. That flailing IS the baseline signal that
  justifies the npx copy change (see the spec).
- `~/tonk/bench` writes are denied by the codex workspace sandbox; the
  agent adapting to its cwd is part of the measured journey.
- `npx tonk` resolves hermetically: npm_config_registry points at the
  run's Caddy-served static registry (see registry.sh).
