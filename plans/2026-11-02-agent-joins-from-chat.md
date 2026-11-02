# Agent joins a space from the chat

Status: in progress, 2026-11-02.

A person pastes a Tonk invite link into a chat with their hosted agent
(Claude, ChatGPT, any MCP Apps host on `/mcp`). The agent shows an ask
card. The card says the agent is the one that joins, and lets the person
choose what the agent may do in the space. The agent joins only when the
person answers in the card.

Mocks: `tonk-labs/gooey` `in-agent/` (the "asks" card, `.tonk--ask`).

## Flow

1. The model calls `tonk_join_space { link }`. The front reads the link
   (expands a `/@/` shortcut, claims a ticket link's ticket, or parses an
   `access=` link) and answers what it offers: the space, its name, and
   whether the link allows write. It changes nothing. The result renders
   the join card (`ui://tonk/join-v1.html`).
2. The card shows the space, "joins as <agent name> bot", and the access
   choice: read, or read and write (only when the link allows write).
3. "join" calls `tonk_join_accept { link, access }`, an app-only tool. The
   front reads the link again and has the link's key delegate to the agent
   space only the scopes of the chosen access
   (`agent_grant_scopes` + `issue_to_recipient`, as the approval page does),
   expiring at the link's own expiry or 90 days, whichever is first. The
   agent space's writer retains the chains and the directory entry
   (`POST /api/access/grant`) and pushes.
4. The card shows the joined strip and tells the model
   (`ui/update-model-context`). The space is now in `tonk_list_spaces`.

## Why the choice holds

- Access is what the retained chains prove. A read-only join holds only
  `/use/get/...` leaves, so the access service refuses a write whatever the
  model asks for.
- The model cannot pick the access: `tonk_join_accept` is app-only, so the
  host does not offer it to the model. A host that ignores app-only
  visibility could let the model call it; it still cannot exceed what the
  link allows, which is what any holder of the link can do.

## Changing and removing access

- An agent that holds read access and gets a link that allows writing is
  offered the upgrade in the card; read stays as it is.
- "Your agents" lists a link's grants beside the person's own, named from
  the agent space's directory. The person may hold no proof for that space,
  so revoking a link's grant drops it from the agent space and queues the
  agent space to sync; the hosted front reads access from there on every
  call. Grants the profile issued are still revoked at the access service.

## Not in this change

- Admin access for an agent (invite others). Agents get read or read and
  write, as on the approval page.
- A roster entry for the agent ("shows as <name> bot" in the space).
- Hosts without MCP Apps (Claude Code CLI): no card, so no join. The tool
  says so.
- Leaving a space from the chat.
