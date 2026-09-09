"""Adapt the onboarding playground to the standard account-aware agent prompt."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LIBRARY = ROOT / 'rust/tonk-core/assets/library'
PAGE = 'concept:CzJUmTs4SBYNXhePYAZGdumwKeg5rnnK4oVv74utdNHd'
PAGE_ENTITY = 'did:key:z6MkF65VFoAVjUMUBsQ7uMzEe5cfxPeJ2M6WZi2ENNqXi4fo'


def adapt_view(view):
    # Replace the old share/join card styling as well as its markup. Both the
    # signed-out message and ready prompt use the same padded surface.
    style_start = view.index('    /* ---- the way in:')
    style_end = view.index('    /* ---- reset:', style_start)
    view = view[:style_start] + '''    /* ---- the way in: one card, present in every state ------------ */
    .pg { width: calc(100% - 32px); }
    .pg .pg-bring {
      margin: 0 0 28px; padding: 20px;
      border: var(--wa-border-width-s) solid var(--wa-color-surface-border);
      background: var(--wa-color-surface-raised);
    }
    .pg h2.pg-bring__title {
      margin: 0 0 12px; padding: 0; background: none;
      font-size: 15px; font-weight: 600; line-height: 1.4;
      text-wrap: balance;
    }
    .pg .playground-agent p {
      margin: 0; max-width: 62ch; line-height: 1.65; text-wrap: pretty;
    }
    .pg .playground-agent .agent-prompt {
      padding: 0; border-radius: 0; background: none; gap: 12px;
    }
    .pg .playground-agent .agent-prompt__action { margin-top: 4px; }
    .pg .pg-status { padding: 16px 20px; margin-bottom: 16px; line-height: 1.5; align-items: baseline; }
    .pg .pg-dot { top: 0; }
    .pg .pg-lede { max-width: 62ch; margin-bottom: 24px; text-wrap: pretty; }
    @media (max-width: 540px) {
      .pg .pg-bring { padding: 16px; }
      .pg .pg-status { padding: 14px 16px; gap: 8px; }
      .pg .pg-said { flex-basis: 100%; margin-left: 17px; }
    }

''' + view[style_end:]
    start = view.index('  <section class="pg-bring"')
    end = view.index('  </section>', start) + len('  </section>')
    return view[:start] + '''  <section class="pg-bring" aria-label="Bring an agent">
    <h2 class="pg-bring__title">Connect an agent</h2>
    <tonk-display model=tonk:blank view=playground-agent></tonk-display>
  </section>''' + view[end:]


def agent_library(core):
    declaration = core.split('concept!: &tonk/agent-invite\n', 1)[1].split('\n# A refused invite', 1)[0]
    declaration = 'concept!: &tonk/agent-invite\n' + declaration
    rule = core.split('rule!:\n  description: Joins the repository name', 1)[1].split('\n# Human-facing invitation', 1)[0]
    rule = 'rule!:\n  description: Joins the repository name' + rule
    view = core.split('view!:\n  this: tonk:agent-invite\n', 1)[1].split('\n# Connection receipt', 1)[0]
    view = 'view!:\n  this: tonk:agent-invite\n' + view
    view = view.replace('Then tell it what you want to build.', 'Then tell it what you want to build on the Agent playground page.')
    view = view.replace('Your agent will connect to this space.', 'Your agent will connect to this space and be instructed to work only on the Agent playground page.')
    view = view.replace("You're helping build the &quot;{name}&quot; Tonk space.", "You're helping build only the &quot;Agent playground&quot; page in the &quot;{name}&quot; Tonk space.")
    view = view.replace('Ask me what I want to build.', 'Ask me what I want to build on the Agent playground page.')
    old = 'Finish with `npx --yes @tonk/cli space home &lt;concept&gt;` to put the result on the space home.'
    assert old in view
    view = view.replace(old, f'''Scope all work to the existing Agent playground page (entity {PAGE_ENTITY}, concept playground-page / {PAGE}). Inspect that page and its playground/* concepts first. Change only its page-specific view, components, and data; use new playground-specific concepts when needed. Preserve the page entity and its place in the navigation. Do not change the space home, other pages, shared components, shared schemas, or space-wide settings. Render the result inside the Agent playground page, not on the space home. If the requested work needs changes outside this page, explain why and ask me first.

      While working here, you may use playground/agent to report your status and playground/deed to record activity. Read their schemas before writing. Their time fields use epoch milliseconds as float literals. The playground checklist is optional; follow what I ask you to build.''')
    body = '\n'.join([declaration, rule, view])
    # The imported space has a legacy agent-invite schema. Keep this variant
    # independent while retaining the current standard handoff attributes.
    body = body.replace('tonk/agent-invite', 'onboarding/agent-invite').replace('tonk:agent-invite', 'tonk:onboarding/agent-invite')
    panel = '''
view!:
  this: tonk:blank
  show:
    playground-agent: |
      <div class="playground-agent">
        <style>.playground-agent tonk-display > [slot][hidden] { display: none !important; }</style>
        <tonk-page on:invite=tonk:agent-handoff></tonk-page>
        <tonk-origin>
          <tonk-display entity={subject} model=tonk:onboarding/agent-invite>
            <tonk-display slot="no-entity" entity={subject} model=tonk:agent-handoff-state>
              <p slot="no-entity">Preparing agent connection…</p>
            </tonk-display>
          </tonk-display>
        </tonk-origin>
      </div>
'''
    return '# Generated by scripts/onboarding/playground.py from core.yaml.\n' + body + panel


if __name__ == '__main__':
    snapshot_path = LIBRARY / 'onboarding.yaml'
    snapshot = json.loads(snapshot_path.read_text())
    for artifact in snapshot['artifacts']:
        if artifact['of'] == PAGE and artifact['the'] == 'xyz.tonk.view/ui':
            artifact['is'] = 'string:' + adapt_view(artifact['is'].removeprefix('string:'))
    snapshot_path.write_text(json.dumps(snapshot, ensure_ascii=False, indent=2) + '\n')
    (LIBRARY / 'onboarding-agent.yaml').write_text(agent_library((LIBRARY / 'core.yaml').read_text()))
