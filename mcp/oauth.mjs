import {createCredentials} from './credentials.mjs';
import { createHash, randomBytes } from 'node:crypto';
import { oauthMetadataResponse } from '@modelcontextprotocol/server';

const random = () => randomBytes(32).toString('base64url');
const hash = value => createHash('sha256').update(value).digest('base64url');
const scope = 'tonk';
const cookieName = '__Host-tonk-link';
const secureHeaders = {
  'cache-control': 'no-store', 'pragma': 'no-cache',
  'referrer-policy': 'no-referrer', 'x-content-type-options': 'nosniff',
};
const json = (body, status = 200, headers = {}) => Response.json(body, {
  status, headers: { ...secureHeaders, ...headers },
});
const error = (name, status = 400) => json({ error: name }, status);
const validOpaque = value => typeof value === 'string' && /^[A-Za-z0-9_-]{43}$/.test(value);

// Fragment grants never enter an HTTP URL or a page interpolation. The callback
// posts only to this origin; cookie + request ID bind it to the browser ceremony.
const callbackScript = `
const fields = new URLSearchParams(location.hash.slice(1));
history.replaceState(null, '', location.pathname + location.search);
const request = new URLSearchParams(location.search).get('request');
const status = document.getElementById('status');
if (!request || fields.has('authorize') === fields.has('deny')) {
  status.textContent = 'This authorization link is incomplete. Start again from ChatGPT.';
} else {
  fetch('/oauth/callback', {
    method: 'POST', credentials: 'same-origin',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ request, authorize: fields.get('authorize'), deny: fields.get('deny') })
  }).then(async response => {
    const result = await response.json();
    if (!response.ok && !result.failure) throw new Error();
    if (result.failure) {
      status.textContent = 'Tonk connection failed at ' + result.failure + '. Start a new connection after the service is fixed.';
      return;
    }
    location.replace(result.redirect);
  }).catch(() => { status.textContent = 'Authorization could not be completed. Start again from ChatGPT.'; });
}`;
const callbackHTML = `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>Connect Tonk</title><body><p id="status" role="status">Completing Tonk authorization…</p><script>${callbackScript}</script></body></html>`;

function httpsURL(value) {
  const url = new URL(value);
  if (url.protocol !== 'https:' || url.username || url.password || url.hash) throw new Error('An uncredentialed HTTPS URL is required.');
  return url;
}
function parameters(params) {
  const output = Object.create(null);
  for (const [key, value] of params) {
    if (Object.hasOwn(output, key)) throw new Error('Duplicate parameter.');
    output[key] = value;
  }
  return output;
}
async function body(request, limit) {
  if (!request.body) return '';
  const reader = request.body.getReader();
  const chunks = [];
  let size = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > limit) { await reader.cancel(); throw new Error('Body exceeds limit.'); }
      chunks.push(value);
    }
    return Buffer.concat(chunks).toString('utf8');
  } finally { reader.releaseLock(); }
}

// Bounded, single-process authorization-code spike. All OAuth state is ephemeral:
// restart invalidates codes and tokens. Native tenant authority stays on disk.
// provisionAccount returns { tenantId, deviceDid, authorize(payload) } and must
// not retain a running process while waiting for browser approval.
export function createTonkOAuth({ issuer, clientId, redirectUris, provisionAccount,
  linkPage = 'https://tonk.network/settings/link', now = Date.now, capacity = 16, onRevoke = () => {}, saved }) {
  const origin = httpsURL(issuer).origin;
  if (issuer !== origin || !clientId || !Number.isSafeInteger(capacity) || capacity < 1) throw new Error('Invalid OAuth configuration.');
  const redirects = new Set(redirectUris.map(uri => { httpsURL(uri); return uri; }));
  if (!redirects.size) throw new Error('Register at least one exact redirect URI.');
  const approvalPage = httpsURL(linkPage);
  const resource = `${origin}/mcp`;
  const metadataURL = `${origin}/.well-known/oauth-protected-resource/mcp`;
  const challenge = `Bearer resource_metadata="${metadataURL}", scope="${scope}"`;
  const metadata = {
    oauthMetadata: {
      issuer, authorization_endpoint: `${origin}/oauth/authorize`, token_endpoint: `${origin}/oauth/token`,
      response_types_supported: ['code'], grant_types_supported: ['authorization_code', 'refresh_token'],
      code_challenge_methods_supported: ['S256'], token_endpoint_auth_methods_supported: ['none'],
      scopes_supported: [scope], authorization_response_iss_parameter_supported: true,
    }, resourceServerUrl: new URL(resource), scopesSupported: [scope], resourceName: 'Tonk',
  };
  const pending = new Map(), codes = new Map(), spent = new Map();
  const credentials = createCredentials({now,capacity,onRevoke,saved:saved?.credentials});
  if(saved){
    if(saved.issuer!==issuer||saved.clientId!==clientId)throw Error('OAuth snapshot configuration mismatch');
    for(const [key,entry] of saved.spent??[])spent.set(key,entry);
  }
  let closed = false;
  const cleanup = () => {
    for (const collection of [pending, codes, spent]) {
      for (const [key, entry] of collection) if (entry.expires <= now()) collection.delete(key);
    }
  };
  function redirectFor(entry, result) {
    const redirect = new URL(entry.redirect);
    for (const [name, value] of Object.entries({ ...result, state: entry.state, iss: issuer })) redirect.searchParams.set(name, value);
    return redirect.href;
  }
  function authenticate(request) {
    cleanup();
    if (closed || new URL(request.url).href !== resource) return undefined;
    const authorization = request.headers.get('authorization');
    if (!authorization?.startsWith('Bearer ') || !validOpaque(authorization.slice(7))) return undefined;
    const principal = credentials.get(authorization.slice(7));
    return principal ? Object.freeze({ ...principal, resource, scope }) : undefined;
  }
  async function fetch(request) {
    cleanup();
    const url = new URL(request.url);
    if (closed) return error('temporarily_unavailable', 503);
    if (url.origin !== origin) return error('invalid_request', 400);
    const discovery = oauthMetadataResponse(request, metadata);
    if (discovery) return discovery;
    try {
      if (url.pathname === '/oauth/authorize' && request.method === 'GET') {
        if (url.search.length > 8192) return error('invalid_request');
        const p = parameters(url.searchParams);
        // Never redirect an invalid client/redirect pair.
        if (p.client_id !== clientId || !redirects.has(p.redirect_uri)) return error('invalid_client');
        if (!p.state || p.state.length > 2048) return error('invalid_request');
        if (p.response_type !== 'code' || p.resource !== resource || p.scope !== scope ||
            p.code_challenge_method !== 'S256' || !validOpaque(p.code_challenge)) {
          return new Response(null, { status: 303, headers: { ...secureHeaders,
            location: redirectFor({ redirect: p.redirect_uri, state: p.state }, { error: 'invalid_request' }) } });
        }
        if (pending.size + codes.size + Math.max(credentials.size(), spent.size) >= capacity) return error('temporarily_unavailable', 503);
        const id = random(), browser = random();
        // Reserve before any async work; concurrent starts obey the capacity.
        const entry = { redirect: p.redirect_uri, state: p.state, challenge: p.code_challenge,
          browser: hash(browser), expires: now() + 300_000 };
        pending.set(id, entry);
        // Identity creation is postponed until callback GET; authorization starts
        // do not allocate native identities merely by being fetched/prefetched.
        const location = `${origin}/oauth/continue?request=${id}`;
        return new Response(null, { status: 303, headers: { ...secureHeaders, location,
          'set-cookie': `${cookieName}-${id}=${browser}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=300` } });
      }
      if (['/oauth/continue', '/oauth/callback'].includes(url.pathname)) {
        // Each attempt owns its cookie: another tab cannot replace its binding.
        const callbackBody = request.method === 'POST'
          ? JSON.parse(await body(request, 180_000)) : undefined;
        const requestId = callbackBody?.request ?? parameters(url.searchParams).request;
        const attemptCookie = `${cookieName}-${requestId}`;
        const cookieParts = (request.headers.get('cookie') ?? '').split(';').map(v => v.trim()).filter(v => v.startsWith(`${attemptCookie}=`));
        const browser = cookieParts.length === 1 ? cookieParts[0].slice(attemptCookie.length + 1) : '';
        const recover = message => {
          if (request.method !== 'GET') return json({ failure: message }, 403);
          const script = "history.replaceState(null, '', location.pathname);";
          return new Response(`<!doctype html><title>Connect Tonk</title><p>${message}</p><script>${script}</script>`, {
            status: 403, headers: { ...secureHeaders, 'content-type': 'text/html; charset=utf-8',
              'content-security-policy': `default-src 'none'; script-src 'sha256-${createHash('sha256').update(script).digest('base64')}'; base-uri 'none'; frame-ancestors 'none'` },
          });
        };
        if (!validOpaque(requestId) || !pending.has(requestId)) return recover('This connection attempt expired or the service restarted. Close this tab and start a new connection from ChatGPT.');
        if (!validOpaque(browser)) return recover('The connection cookie is missing. Start a new connection from ChatGPT in this browser.');
        if (request.method === 'GET') {
          const id = parameters(url.searchParams).request;
          const entry = pending.get(id);
          if (entry.browser !== hash(browser)) return recover('This connection belongs to a different browser session. Start a new connection from ChatGPT.');
          if (url.pathname === '/oauth/continue') {
            if (entry.started) return error('invalid_request');
            entry.started = true;
            // The adapter closes the native process after reading its identity.
            const account = await provisionAccount();
            if (typeof account.tenantId !== 'string' || !account.tenantId ||
                typeof account.deviceDid !== 'string' || !account.deviceDid.startsWith('did:key:')) return error('invalid_request');
            entry.account = account;
            if (closed || !pending.has(id) || entry.expires <= now()) return error('invalid_request');
            const approval = new URL(approvalPage);
            approval.searchParams.set('audience', account.deviceDid);
            approval.searchParams.set('callback', `${origin}/oauth/callback?request=${id}`);
            approval.searchParams.set('name', 'Tonk for ChatGPT');
            return new Response(null, { status: 303, headers: { ...secureHeaders, location: approval.href } });
          }
          return new Response(callbackHTML, { headers: { ...secureHeaders, 'content-type': 'text/html; charset=utf-8',
            'content-security-policy': `default-src 'none'; script-src 'sha256-${createHash('sha256').update(callbackScript).digest('base64')}'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'` } });
        }
        if (url.pathname !== '/oauth/callback' || request.method !== 'POST' ||
            request.headers.get('origin') !== origin || request.headers.get('content-type') !== 'application/json') return error('invalid_request', 403);
        const p = callbackBody;
        const entry = pending.get(p.request);
        if (!entry?.account || entry.processing || entry.browser !== hash(browser)) return error('invalid_request', 403);
        // Mark consumed before awaiting native activation. Keep its capacity slot
        // until completion so parallel callbacks cannot bypass admission bounds.
        entry.processing = true;
        const clearCookie = { 'set-cookie': `${attemptCookie}=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0` };
        if (typeof p.deny === 'string' && p.authorize == null) {
          pending.delete(p.request);
          return json({ redirect: redirectFor(entry, { error: 'access_denied' }) }, 200, clearCookie);
        }
        if (typeof p.authorize !== 'string' || p.deny != null || !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(p.authorize)) return error('invalid_request');
        let identity;
        try { identity = await entry.account.authorize(JSON.parse(Buffer.from(p.authorize, 'base64').toString('utf8'))); }
        catch (cause) {
          pending.delete(p.request);
          const stages = ['provider', 'worker-start', 'device-check', 'root-import', 'account-attach', 'account-hydration', 'checkpoint'];
          const failure = stages.includes(cause?.authorizationStage) ? cause.authorizationStage : 'account-activation';
          return json({ failure }, 200, clearCookie);
        }
        if (closed || entry.expires <= now() || identity.deviceDid !== entry.account.deviceDid ||
            typeof identity.rootDid !== 'string' || !identity.rootDid.startsWith('did:key:')) return error('invalid_request');
        const code = random();
        pending.delete(p.request);
        codes.set(hash(code), { ...entry, account: undefined, expires: now() + 60_000,
          principal: { tenantId: entry.account.tenantId, deviceDid: identity.deviceDid, rootDid: identity.rootDid } });
        return json({ redirect: redirectFor(entry, { code }) }, 200, clearCookie);
      }
      if (url.pathname === '/oauth/token' && request.method === 'POST') {
        if (request.headers.has('authorization') || request.headers.get('content-type')?.split(';')[0] !== 'application/x-www-form-urlencoded') return error('invalid_request');
        const p = parameters(new URLSearchParams(await body(request, 8192)));
        if (p.client_id !== clientId || p.client_secret !== undefined || p.client_assertion !== undefined) return error('invalid_client');
        if ((p.resource !== undefined && p.resource !== resource) || (p.scope !== undefined && p.scope !== scope)) return error('invalid_request');
        const tokenResponse = value => json({access_token:value.token,refresh_token:value.refreshToken,token_type:'Bearer',expires_in:Math.floor((value.expiresAt-now())/1000),scope});
        if (p.grant_type === 'refresh_token') {
          const renewed = credentials.renew(p.refresh_token);
          return renewed ? tokenResponse(renewed) : error('invalid_grant');
        }
        if (p.grant_type !== 'authorization_code' || p.resource !== resource) return error('invalid_request');
        const used = validOpaque(p.code) ? spent.get(hash(p.code)) : undefined;
        if (used && used.redirect === p.redirect_uri && hash(p.code_verifier ?? '') === used.challenge) {
          credentials.revokeHash(used.tokenHash);
          return error('invalid_grant');
        }
        const entry = validOpaque(p.code) ? codes.get(hash(p.code)) : undefined;
        if (!entry || entry.redirect !== p.redirect_uri || !/^[A-Za-z0-9._~-]{43,128}$/.test(p.code_verifier ?? '') || hash(p.code_verifier) !== entry.challenge) return error('invalid_grant');
        codes.delete(hash(p.code));
        const value = credentials.issue({...entry.principal,connectionExpiresAt:now()+8*60*60_000});
        spent.set(hash(p.code), { tokenHash:createHash('sha256').update(value.token).digest('hex'), redirect:entry.redirect, challenge:entry.challenge, expires:value.expiresAt });
        return tokenResponse(value);
      }
      return error('not_found', 404);
    } catch { return error('invalid_request'); }
  }
  return { fetch, authenticate, resource,
    retainedTenants(){
      cleanup();
      return new Set([...pending.values()].map(entry=>entry.account?.tenantId)
        .concat([...codes.values()].map(entry=>entry.principal.tenantId),credentials.snapshot().map(entry=>entry.value.tenantId)).filter(Boolean));
    },
    snapshot(){cleanup();return {issuer,clientId,credentials:credentials.snapshot(),spent:[...spent]};},
    challenge: () => json({ error: 'invalid_token' }, 401, { 'www-authenticate': challenge }),
    close() { closed = true; pending.clear(); codes.clear(); credentials.close(); spent.clear(); },
  };
}
