export function connectionSettings(config) {
  const clients = JSON.parse(config.vars.TONK_MCP_CLIENTS);
  const rows = clients.map((client) => {
    const callbacks = client.redirectUris.map((uri) => `\`${uri}\``).join('<br>');
    return `| ${client.displayName} | \`${client.clientId}\` | ${callbacks} |`;
  });
  return [
    `MCP URL (shared by all clients): \`${config.vars.TONK_MCP_ISSUER}/mcp\``,
    '',
    'Connect using Streamable HTTP with OAuth authorization code + PKCE. Leave the OAuth client secret empty (public clients). Use the matching client ID and callback below.',
    '',
    '| Client | OAuth client ID | Registered callback |',
    '| --- | --- | --- |',
    ...rows,
    '',
    '- **ChatGPT / Claude web:** add a custom MCP connector with the shared URL and matching OAuth client ID. Live login, tools and rendering still require manual verification.',
    '- **Native harnesses:** use the native client ID and a literal loopback callback above; an arbitrary local port is allowed, for example `http://127.0.0.1:54321/callback`. The harness must support configuring that client ID and callback.',
    '- **Claude Code:** not yet supported by this callback policy; its documented `http://localhost:PORT/callback` is rejected. Setting the port alone does not resolve this.',
    '- **Other harnesses:** require an explicitly registered matching callback. URL-only dynamic registration is disabled in this deployment; client ID metadata documents (CIMD) are not supported.',
    '',
    'Automated discovery, authentication challenge and asset checks passed. These checks do not prove live client OAuth, tool use or UI rendering.',
  ].join('\n');
}
