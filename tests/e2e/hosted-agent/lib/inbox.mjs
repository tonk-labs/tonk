// Where the test account's activation link is read back from.
//
// Against a deployed preview the account is a Resend test address, which
// Resend accepts and never delivers, and the link is read through Resend's
// API. Against a local `tonk-access-local` the service captures the email
// itself and serves it at `/_test/emails`.

const RESEND = 'https://api.resend.com';
const LINK = /https?:\/\/[^\s"'<>]+\/activate[^\s"'<>]*/;

/** An inbox for one run, with the address to sign up with. */
export function inbox({ run, resendKey, captured }) {
  if (captured) {
    return {
      address: `e2e-${run}@example.com`,
      async link(address) {
        return poll(async () => {
          const response = await fetch(new URL('/_test/emails', captured));
          const emails = await response.json();
          return emails.findLast(([to]) => to === address)?.[1];
        }, `an activation email for ${address} at ${captured}`);
      },
    };
  }
  if (!resendKey) {
    throw new Error('set TONK_E2E_RESEND_API_KEY, or TONK_E2E_CAPTURED_EMAILS for a local service');
  }
  const headers = { authorization: `Bearer ${resendKey}` };
  return {
    address: `delivered+e2e-${run}@resend.dev`,
    async link(address) {
      return poll(async () => {
        const response = await fetch(`${RESEND}/emails?limit=100`, { headers });
        if (!response.ok) {
          throw new Error(`Resend listed emails with ${response.status}: ${await response.text()}`);
        }
        const { data } = await response.json();
        const sent = data.find((email) => email.to.includes(address));
        if (!sent) return undefined;
        const email = await (await fetch(`${RESEND}/emails/${sent.id}`, { headers })).json();
        const link = (email.html ?? email.text ?? '').match(LINK)?.[0];
        return link?.replaceAll('&amp;', '&');
      }, `an activation email for ${address} in Resend`);
    },
  };
}

async function poll(look, what, timeout = 60_000) {
  const deadline = Date.now() + timeout;
  for (;;) {
    const found = await look();
    if (found) return found;
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await new Promise((done) => setTimeout(done, 1000));
  }
}
