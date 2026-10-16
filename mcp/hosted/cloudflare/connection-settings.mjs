export function connectionSettings(config) {
  return [
    `**Server:** \`${config.vars.TONK_MCP_ISSUER}/mcp\``,
    'Streamable HTTP · no OAuth configuration',
    '',
    'Copy an agent link from the desired Tonk space and supply the complete link as',
    '`agent_link` on every `tonk_query` or `tonk_evaluate` call, alongside `document`.',
    'The link includes private credentials. Share it only with the intended agent.',
  ].join('\n');
}
