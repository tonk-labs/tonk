export function connectionSettings(config) {
  const clients = JSON.parse(config.vars.TONK_MCP_CLIENTS);
  const labels = {
    'Tonk for ChatGPT': 'ChatGPT',
    'Tonk for Claude': 'Claude web',
    'Tonk native client': 'Native',
  };
  const rows = clients.map((client) => {
    const callbacks = client.redirectUris
      .map((uri) => {
        const callback = new URL(uri);
        const display =
          client.applicationType === 'native' && callback.protocol === 'http:'
            ? `${callback.protocol}//${callback.hostname}:PORT${callback.pathname}${callback.search}`
            : uri;
        return `\`${display}\``;
      })
      .join(' or ');
    return `| ${labels[client.displayName] ?? client.displayName} | \`${client.clientId}\` | ${callbacks} |`;
  });
  return [
    `**Server:** \`${config.vars.TONK_MCP_ISSUER}/mcp\`  `,
    'Streamable HTTP · OAuth + PKCE · leave client secret empty',
    '',
    '| Client | Client ID | Callback |',
    '| --- | --- | --- |',
    ...rows,
  ].join('\n');
}
