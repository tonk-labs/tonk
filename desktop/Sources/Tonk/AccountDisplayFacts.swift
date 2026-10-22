import Foundation

struct AccountPasskey: Identifiable, Equatable {
  let id: String
  let createdOn: String
  let createdAt: Date?
}

/// Read the same profile facts used by the browser account page. Only display
/// metadata crosses the bridge: never ciphertext, credential IDs or custody keys.
enum AccountDisplayFacts {
  static let script = """
    const variable = name => ({'?': {name}});
    const field = (the, as) => ({the, as, cardinality: 'one'});
    const meta = '/api/profile/branch/meta/query';
    const active = await api(meta, {
      predicate: {with: {branch: field('tonk.dialog.replica/active-branch', 'Entity')}},
      terms: {this: variable('this'), branch: variable('branch')}
    });
    let branch = 'main';
    if (active[0]?.fields?.branch) {
      const names = await api(meta, {
        predicate: {with: {name: field('xyz.tonk.branch/name', 'Text')}},
        terms: {this: active[0].fields.branch, name: variable('name')}
      });
      if (!names[0]?.fields?.name) throw new Error('Cannot resolve the account branch.');
      branch = names[0].fields.name;
    }
    const path = '/api/profile/branch/' + encodeURIComponent(branch) + '/query';
    const [registration, envelopes] = await Promise.all([
      api(path, {
        predicate: {with: {email: field('xyz.tonk.account/customer-email', 'Text')}},
        terms: {this: root.rootDid, email: variable('email')}
      }),
      api(path, {
        predicate: {with: {to: field('xyz.tonk.secret/to', 'Entity'),
          sender: field('xyz.tonk.secret/sender', 'Entity')}},
        terms: {this: variable('this'), to: variable('to'), sender: root.rootDid}
      })
    ]);
    if (!Array.isArray(registration) || !Array.isArray(envelopes))
      throw new Error('Invalid account facts response.');
    const recipients = [...new Set(envelopes.map(row => row.fields?.to).filter(Boolean))];
    const rows = (await Promise.all(recipients.map(recipient => api(path, {
      predicate: {with: {createdOn: field('xyz.tonk.recovery/created-on', 'Text'),
        createdAt: field('xyz.tonk.recovery/created-at', 'UnsignedInteger')}},
      terms: {this: recipient, createdOn: variable('createdOn'), createdAt: variable('createdAt')}
    })))).flat();
    const passkeys = rows.map((row, index) => ({
      id: String(index), createdOn: row.fields.createdOn, createdAt: row.fields.createdAt
    })).sort((a,b) => b.createdAt - a.createdAt);
    return {email: registration[0]?.fields?.email || null, passkeys};
    """
}
