// Kept free of the Containers runtime so it can be tested under plain Node.
export const guestAssetPaths = ['/space-runtime.json', '/worker-guest.html'];

export async function guestAsset(request, assets) {
  const asset = await assets.fetch(request);
  const response = new Response(asset.body, asset);
  response.headers.set('Access-Control-Allow-Origin', '*');
  response.headers.set('X-Content-Type-Options', 'nosniff');
  // A subdomain may claim a parent domain as its WebAuthn RP ID, so a page
  // here could otherwise assert passkeys registered on the apex. Guest
  // pages never need WebAuthn; disable it for them and their frames.
  response.headers.set(
    'Permissions-Policy',
    'publickey-credentials-get=(), publickey-credentials-create=()',
  );
  return response;
}
