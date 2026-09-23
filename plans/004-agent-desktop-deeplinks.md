# Open the Tonk agent prompt in a desktop app

Status: IMPLEMENTED — CODEX AND CLAUDE ENABLED. The product implementation and
automated contract checks passed on 2026-09-21. Claude is enabled optimistically;
its eligible-account GUI validation remains a release follow-up.

## Implementation evidence

- The canonical `tonk-agent-prompt` builds the Codex and Claude Code destinations
  from the exact final copy prompt after invitation and localhost substitutions.
  It removes the destinations while bindings are stale, unsupported, or above
  the 12,000 UTF-16 code-unit cap; the full copy prompt remains available at the
  cap fallback.
- Tonk's sealed-guest navigation policy admits only the fixed
  `codex://new?prompt=...` and `claude://code/new?q=...` route shapes. Other
  custom destinations and malformed variants remain rejected, without logging
  the prompt or destination.
- The generated onboarding-agent asset exactly matches `agent_library(core)`
  and retains its playground-specific prompt. Storybook source and generated
  data describe the shipped Codex and Claude Code actions.
- The focused Chrome invitation E2E passed against the built connection-enabled
  artifact with `TONK_TEST_WEB_HOST=localhost`. It covers exact copied/decoded
  prompt equality, localhost and hosted forms, quotes, newlines, Unicode and
  query characters, invitation replacement, stale-link removal, 12,000/12,001
  boundaries, narrow layout, copy behaviour, and the existing real CLI join and
  receipt flow. The onboarding-playground browser case passed separately.
- Host/guest Wasm tests passed 80/80. Focused standard-library tests, Rust format
  and check, Storybook generation/link checks, and `git diff --check` also passed.

Actual product-card clicks into external desktop apps remain manual release
checks. Codex cold launch, Chrome's operating-system handoff, Claude Code with
an eligible account, Windows/Linux/mobile, and a
submitted disposable-space agent session were not run in this implementation
pass. The earlier inert Safari Codex warm-launch spike remains the only verified
external composer handoff.

## Intended experience

The ready invitation card offers `open in Codex`, `open in Claude Code`, and
`copy prompt`. A desktop link opens an unsent composer containing exactly the
same prompt as the copy control. The user reviews it and sends it. Existing
`tonk join` receipt handling remains the sole connection confirmation.

Human copy: `open this prompt in your agent or copy it. review it before sending,
then tell it what you want to build.` Preserve the generated playground-specific
wording. Keep copy first in the action row, followed by Codex and Claude Code.
Do not promise one click for installation, sign-in, workspace choice, browser
permission, or account eligibility. Claude Code means the Code surface in
Claude Desktop, not an ordinary Claude chat.

## Spike evidence and limits

- Safari on macOS opened a loopback HTML fixture with real custom-scheme anchors.
  Safari asked permission to launch each app; only the one-time Allow was used.
- Fixture: current onboarding prompt, dummy space name with quotes/ampersand/
  Unicode, an 8,192-character fake grants payload, dummy v2 fragment, and boundary
  markers. The invitation uses `example.invalid` and grants no real authority.
- Prompt: 10,703 characters. Claude URL: 11,745 characters. Codex URL: 11,744.
  The payload is a transport stress fixture, not a valid signed invitation or
  a measured maximum of real production invitations.
- Codex protocol handler on this machine is ChatGPT.app 26.915.31945 (9922).
  The user confirmed a new unsent draft containing both start and end markers,
  including the final `# & + % ? café 日本語` text. Computer Use explicitly blocks
  inspecting `com.openai.codex`, so automated whole-composer equality was not
  checked. Codex was already running; cold launch remains untested.
- Claude.app 2.2553.1 launched, but the user is signed out and lacks a paid plan
  for Code GUI access. Composer preservation and Code routing remain unverified.
  Do not treat opening the app alone as passing the handoff.
- No prompt was submitted, no agent connected to Tonk, and no live invitation
  was minted. No Chrome, Windows, Linux, mobile, installed PWA, or in-Tonk
  navigation check was performed.

Reproduce without credentials:

```sh
python3 scripts/onboarding/deeplink_spike.py /tmp/tonk-deeplink-spike
python3 -m http.server 8765 --bind 127.0.0.1 --directory /tmp/tonk-deeplink-spike
```

Open http://127.0.0.1:8765 in a browser and click the test links. Do not send the
dummy prompt. The generator asserts full query-parameter round-trip equality;
`prompt.txt` holds the expected text for manual comparison.

## Supported contract

- `codex://new?prompt=<encodeURIComponent(prompt)>` opens a local chat with
  unsent composer text. Optional `path` requires an actual absolute local path;
  `originUrl` matches a Git remote, not a Tonk space.
- `claude://code/new?q=<encodeURIComponent(prompt)>` opens the Code composer.
  Claude documents truncation at roughly 14,000 prompt characters. Optional
  `folder` requires an absolute local path and triggers folder confirmation.
- Omit workspace parameters: a Tonk space name is not a filesystem directory.
- Claude CLI has `claude-cli://open?q=...`, but terminal support is deferred
  from this GUI increment. There is no verified Codex terminal URL contract.

Sources checked during exploration:

- https://learn.chatgpt.com/docs/reference/commands
- https://support.claude.com/en/articles/14729294-open-claude-desktop-with-a-link
- https://code.claude.com/docs/en/deep-links

## Increment 1: one prompt and safe URL construction

Inspect current instructions, worktree changes, and these sources first:

- `rust/tonk-core/assets/library/core.yaml`: canonical `tonk-agent-prompt` module
  and standard invitation view.
- `scripts/onboarding/playground.py`: `agent_library(core)` derives the
  playground view, including its stricter page-only prompt instructions.
- `rust/tonk-core/assets/library/onboarding-agent.yaml`: generated result.
- `rust/tonk-ui/src/account_flow.rs`: existing browser invitation tests, notably
  `it_connects_with_an_ordinary_bearer_after_the_issuer_closes`.

Add URL construction from the final copy prompt, including localhost's `tonk`
and `TONK_CONNECTION_ORIGIN` substitution. Avoid independent prompt templates.
Both core and onboarding register the same custom-element name; keep their
shared implementation identical regardless of registration order.

Keep links unavailable until the invitation is ready and its prompt bindings
are current. Recompute on name/link/prompt changes, including a new invite;
never retain a stale bearer URL. Audit attribute updates as well as child
mutations: the current observer only watches childList/subtree. Guard attribute
writes by equality to avoid observer loops. Hide/remove stale destinations when
the invitation becomes unsupported or unavailable.

Use fixed app scheme/host/path and encode the entire prompt once. Construct
locally with no redirect service, storage, analytics URL capture, or automatic
clipboard copying. Do not log the prompt or destination. Keep the existing
invitation privacy notice and all scope/receipt instructions.

Use a conservative initial prompt cap of 12,000 UTF-16 code units for both app
links, below Claude's approximate limit; this is a product guard, not a proven
cross-platform URL maximum. Above it, retain the full copy prompt and explain
that it must be pasted. Never truncate. Tonk's envelope parser allows much larger
invitations, and shortening can fail back to a long URL, so do not assume the
8 KB stress fixture bounds production input.

Focused proof: decoded URL prompt equals copied prompt for ordinary and
playground text, localhost and hosted origins, quotes/newlines/Unicode/query
characters, invitation replacement, and length-boundary fallback. Prefer an
existing browser-test harness that exercises the actual component.

## Increment 2: controls and generated onboarding

Add the desktop actions alongside copy in the canonical card. Use accessible
names, visible keyboard focus, at least 44px targets, and wrapping on narrow
screens. Ordinary anchors are the first candidate; verify that Tonk's own
navigation/portal handling permits these schemes on a user click. Avoid hidden
iframe launches, install detection, timers that claim success, or auto-submit.

Regenerate only the onboarding-agent asset with `agent_library(core)`. The
script's full main also rewrites the onboarding snapshot; inspect that path
before invoking it and avoid unrelated snapshot changes. Verify that the
generated file exactly matches the generator output and that playground-only
scope instructions survive.

Update the existing browser assertion that the action row contains exactly one
action: it currently counts buttons, copy controls, and links. Test the intended
controls, no overflow, and continued copy/receipt behaviour instead of retaining
that obsolete count. Keep `new invite` separate from the launch actions.

Do not ship the Claude action as verified until an eligible account passes its
GUI check; Codex can ship independently if that remains blocked.

## Integration and release gate

Run the focused invitation browser tests through repository-defined commands.
Check the repository's Storybook impact workflow if these assets trigger it;
update relevant stories/journeys and generated assets through that workflow.
Run formatting applicable to changed files and `git diff --check`. Run Rust
formatting/tests if Rust tests or source change; do not claim native tests prove
external app handling.

Manually validate from the actual ready Tonk card in Safari and Chrome:

1. Codex warm and cold launch; composer stays unsent and full text matches copy.
2. Claude Code with eligible account; correct Code surface, unsent full prompt.
3. Default/no folder selection and narrow viewport.
4. Hosted and localhost prompts, fresh invite replacement, long-prompt fallback.
5. A separately authorized disposable-space connection through the real agent:
   submitting the prompt runs join and the existing receipt updates Tonk. This
   final integration is separate from the inert spike and must not use dummy data.

Record app versions, platforms, prompt lengths, observed permissions, and
remaining gaps. No automatic agent submission or new authority protocol is
needed. Keep increments independently reviewable and update this plan with
fresh evidence after implementation.
