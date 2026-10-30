// Where the test account's activation link is read back from: the access
// service's `/_test/emails`. A preview records mail to `.test` addresses
// there instead of sending it (rust/tonk-access-service/src/handlers/
// test_inbox.rs); a local `tonk-access-local` records every email.

/** An inbox for one run at `base`, with the address to sign up with. */
export function inbox({ run, base }) {
  return {
    address: `e2e-${run}@hosted-agent.test`,
    async link(address) {
      const url = new URL('/_test/emails', base);
      url.searchParams.set('address', address);
      return poll(async () => {
        const response = await fetch(url);
        if (!response.ok) {
          throw new Error(`${url} answered ${response.status}: ${await response.text()}`);
        }
        const emails = await response.json();
        return emails.findLast(([to]) => to === address)?.[1];
      }, `an activation email for ${address} at ${base}`);
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
