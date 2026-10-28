export function connectionSettings(config) {
  const issuer = config.vars.TONK_MCP_ISSUER;
  return [
    `**OAuth server (ChatGPT, Claude):** \`${issuer}/mcp\``,
    'Streamable HTTP · OAuth with dynamic registration; no client ID needed.',
    'Sign in to Tonk and approve to sponsor the agent: it gets its own identity, and can',
    'create spaces you own and work in them. Open the claim link it gives you to add a',
    'space to your Tonk space list.',
    '',
    `**Agent links:** \`${issuer}/mcp/link\``,
    'Streamable HTTP · no OAuth. Copy an agent link from the desired Tonk space and',
    'supply the complete link as `agent_link` on every `tonk_query` or `tonk_evaluate`',
    'call, alongside `document`. The link includes private credentials. Share it only',
    'with the intended agent.',
  ].join('\n');
}
