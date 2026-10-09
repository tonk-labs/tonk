// Registration must be able to discover the issuer before ChatGPT supplies
// its exact callback URI. This exposes no tenant state and issues no tokens.
export function discovery(request, issuer) {
  const url = new URL(request.url);
  if (url.origin !== issuer) return new Response(null, { status: 421 });
  const headers = { 'cache-control': 'no-store' };
  if (request.method === 'GET' && url.pathname === '/.well-known/oauth-authorization-server') {
    return Response.json({ issuer, authorization_endpoint: `${issuer}/oauth/authorize`,
      token_endpoint: `${issuer}/oauth/token`, response_types_supported: ['code'],
      grant_types_supported: ['authorization_code', 'refresh_token'], code_challenge_methods_supported: ['S256'],
      token_endpoint_auth_methods_supported: ['none'], scopes_supported: ['tonk'],
      authorization_response_iss_parameter_supported: true }, { headers });
  }
  if (request.method === 'GET' && url.pathname === '/.well-known/oauth-protected-resource/mcp') {
    return Response.json({ resource: `${issuer}/mcp`, authorization_servers: [issuer],
      scopes_supported: ['tonk'], resource_name: 'Tonk' }, { headers });
  }
  if (url.pathname === '/mcp') return new Response(null, { status: 401, headers: {
    ...headers, 'www-authenticate': `Bearer resource_metadata="${issuer}/.well-known/oauth-protected-resource/mcp", scope="tonk"`,
  } });
  return new Response('Tonk test connection is awaiting its registered OAuth callback.', { status: 503, headers });
}
