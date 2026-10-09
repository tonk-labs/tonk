import { Container, ContainerProxy } from '@cloudflare/containers';
import { outbound } from './outbound.mjs';
import { discovery } from './discovery.mjs';
export { ContainerProxy };

export class TonkRuntime extends Container {
  defaultPort = 8080;
  sleepAfter = '20m';
  enableInternet = false;
  interceptHttps = true;
  allowedHosts = ['tonk.foundation', 'checkpoint.internal'];
  constructor(ctx, env) {
    super(ctx, env);
    this.envVars = {
      TONK_MCP_BINARY: '/usr/local/bin/tonk-mcp-runtime',
      TONK_MCP_ISSUER: env.TONK_MCP_ISSUER,
      TONK_MCP_SPACE: env.TONK_MCP_SPACE,
      TONK_MCP_CLIENT_ID: env.TONK_MCP_CLIENT_ID,
      TONK_MCP_REDIRECT_URI: env.TONK_MCP_REDIRECT_URI,
    };
  }
}
TonkRuntime.outbound = outbound;

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (!env.TONK_MCP_ISSUER) {
      return new Response('Tonk test connection is not configured.', { status: 503 });
    }
    if (url.origin !== env.TONK_MCP_ISSUER) return new Response(null, { status: 421 });
    if (['/space-runtime.json', '/space-guest.html', '/worker-guest.html'].includes(url.pathname) && ['GET', 'HEAD'].includes(request.method)) {
      const asset = await env.ASSETS.fetch(request);
      const response = new Response(asset.body, asset);
      response.headers.set('Access-Control-Allow-Origin', '*');
      response.headers.set('X-Content-Type-Options', 'nosniff');
      return response;
    }
    if (!env.TONK_MCP_SPACE || !env.TONK_MCP_CLIENT_ID || !env.TONK_MCP_REDIRECT_URI) {
      return discovery(request, env.TONK_MCP_ISSUER);
    }
    if (!['/mcp', '/oauth/authorize', '/oauth/continue', '/oauth/callback', '/oauth/token',
      '/.well-known/oauth-protected-resource/mcp', '/.well-known/oauth-authorization-server'].includes(url.pathname) && !url.pathname.startsWith('/space-api/')) {
      return new Response(null, { status: 404 });
    }
    // One named instance: OAuth state and private checkpoint cannot be spread
    // across random containers. The public caller cannot choose the instance.
    return env.RUNTIME.getByName('chatgpt-single-space-v1').fetch(request);
  },
};
