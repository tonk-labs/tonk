import { Container, ContainerProxy } from '@cloudflare/containers';
import { outbound } from './outbound.mjs';
import { guestAsset, guestAssetPaths } from './guest-assets.mjs';
import { providerOrigin } from '../provider.mjs';
export { ContainerProxy };

export class TonkRuntime extends Container {
  defaultPort = 8080;
  sleepAfter = '20m';
  enableInternet = false;
  interceptHttps = true;
  constructor(ctx, env) {
    super(ctx, env);
    this.allowedHosts = [
      new URL(providerOrigin(env.TONK_PROVIDER_ORIGIN)).hostname,
      'checkpoint.internal',
    ];
    this.envVars = {
      TONK_PROVIDER_ORIGIN: providerOrigin(env.TONK_PROVIDER_ORIGIN),
      TONK_MCP_ISSUER: env.TONK_MCP_ISSUER,
    };
  }
}
TonkRuntime.outbound = outbound;

// Paths the container serves: both MCP endpoints, OAuth discovery and the
// authorization server.
const containerPaths = new Set([
  '/mcp',
  '/mcp/link',
  '/build.json',
  '/.well-known/oauth-protected-resource/mcp',
  '/.well-known/oauth-authorization-server',
  '/oauth/authorize',
  '/oauth/continue',
  '/oauth/callback',
  '/oauth/token',
  '/oauth/revoke',
  '/oauth/register',
]);
export const containerPath = (path) =>
  containerPaths.has(path) || /^\/oauth\/register\/[A-Za-z0-9_-]{1,64}$/.test(path);

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (!env.TONK_MCP_ISSUER) {
      return new Response('Tonk test connection is not configured.', { status: 503 });
    }
    if (url.origin !== env.TONK_MCP_ISSUER) return new Response(null, { status: 421 });
    if (guestAssetPaths.includes(url.pathname) && ['GET', 'HEAD'].includes(request.method)) {
      return guestAsset(request, env.ASSETS);
    }
    if (!env.TONK_PROVIDER_ORIGIN) return new Response(null, { status: 503 });
    if (!containerPath(url.pathname)) return new Response(null, { status: 404 });
    // Separate instance and checkpoint namespace from the former OAuth service.
    return env.RUNTIME.getByName('agent-link-v1').fetch(request);
  },
};
