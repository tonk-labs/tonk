import Foundation
import HarnessCore

@MainActor
extension RuntimeModel {
  func refreshSpaces() async {
    guard !catalogLoading else {
      catalogRefreshPending = true
      return
    }
    catalogLoading = true
    catalogError = nil
    defer {
      catalogLoading = false
      if catalogRefreshPending {
        catalogRefreshPending = false
        Task { await refreshSpaces() }
      }
    }
    do {
      let result = try await accountScript(
        """
        const account = await api('/api/account');
        const identity = await api('/api/identity/root');
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
          if (!names[0]?.fields?.name) throw new Error('Cannot resolve the active account branch.');
          branch = names[0].fields.name;
        }
        const path = '/api/profile/branch/' + encodeURIComponent(branch) + '/query';
        const query = {
          predicate: {with: {
            subject: field('xyz.tonk.space/subject', 'Entity'),
            name: {...field('xyz.tonk.space/name', 'Text'), optional: true},
            icon: {...field('xyz.tonk.space/icon', 'Text'), optional: true},
            color: {...field('xyz.tonk.space/color', 'Text'), optional: true}
          }},
          terms: {this: variable('this'), subject: variable('subject'), name: variable('name'),
                  icon: variable('icon'), color: variable('color')}
        };
        const rows = await api(path, query);
        \(RuntimeCatalogObservation.script)
        if (!Array.isArray(rows)) throw new Error('Invalid space catalog response.');
        return {branch, root: identity.rootDid || '', status: account.status, spaces: rows.map(row => ({subject: row.fields.subject, name: row.fields.name || null,
          icon: row.fields.icon || null, color: row.fields.color || null}))};
        """, arguments: ["catalogGeneration": catalogObservation.generation])
      try Task.checkCancellation()
      guard let rows = result["spaces"] else {
        throw CallbackError("The runtime returned no space catalog.")
      }
      let decoded = try TonkSpace.decodeCatalog(JSONSerialization.data(withJSONObject: rows))
      guard let branch = result["branch"] as? String, let root = result["root"] as? String,
        let status = result["status"] as? String
      else { throw CallbackError("Invalid catalog account context.") }
      try acceptCatalog(decoded, branch: branch, root: root, status: status)
      if !spaces.isEmpty { catalogRecoveryStarted = nil }
      if let selectedSpace {
        self.selectedSpace = spaces.first(where: { $0.id == selectedSpace.id })
      }
      catalogBranch = result["branch"] as? String
      catalogLoaded = true
    } catch is CancellationError {
      return
    } catch {
      catalogError =
        (error as NSError).userInfo["WKJavaScriptExceptionMessage"] as? String
        ?? error.localizedDescription
    }
  }

  /// Sets or clears the account's sidebar icon and tint for `space`. They live on
  /// the account directory, so every desktop on the account picks them up.
  func setSpaceAppearance(_ space: TonkSpace, icon: String?, color: String?) async throws {
    guard let branch = catalogBranch else {
      throw HarnessError.message("Spaces are still loading. Try again in a moment.")
    }
    _ = try await accountScript(
      """
      const base = '/api/profile/branch/' + encodeURIComponent(branch);
      const text = (key, the) => ({kind: 'durable', concept: {with: {
        [key]: {the, as: 'Text', cardinality: 'one'}}}});
      const field = (the, as) => ({the, as, cardinality: 'one'});
      // Read the stored values now, so clearing retracts exactly what is there.
      const rows = await api(base + '/query', {
        predicate: {with: {
          subject: field('xyz.tonk.space/subject', 'Entity'),
          icon: {...field('xyz.tonk.space/icon', 'Text'), optional: true},
          color: {...field('xyz.tonk.space/color', 'Text'), optional: true}
        }},
        terms: {this: subject, subject: {'?': {name: 'subject'}},
                icon: {'?': {name: 'icon'}}, color: {'?': {name: 'color'}}}
      });
      const stored = rows[0]?.fields || {};
      const claims = [];
      const set = (key, the, value, current) => {
        if (value) {
          claims.push({op: 'assert', application: {
            predicate: text(key, the), parameters: {this: subject, [key]: value}}});
        } else if (current) {
          claims.push({op: 'retract', application: {
            predicate: text(key, the), parameters: {this: subject, [key]: current}}});
        }
      };
      set('icon', 'xyz.tonk.space/icon', icon, stored.icon);
      set('color', 'xyz.tonk.space/color', color, stored.color);
      if (claims.length) await api(base + '/transact', {claims});
      return {saved: claims.length};
      """,
      arguments: [
        "branch": branch, "subject": space.subject,
        "icon": icon ?? "", "color": color ?? "",
      ])
    await refreshSpaces()
  }

  func openSpace(_ space: TonkSpace, at url: URL? = nil) {
    spaceLoading = true
    selectedSpace = space
    if selectedView?.url(in: space) != url { selectedView = nil }
    spaceError = nil
    let target = url.flatMap { space.contains($0) ? $0 : nil } ?? space.url
    spaceWebView.load(URLRequest(url: target))
  }

  func showSpaces() {
    closeSpace()
    Task { await refreshSpaces() }
  }

  /// Leaves the space window's page blank, so a later space never flashes the last one.
  func closeSpace() {
    selectedSpace = nil
    selectedView = nil
    spaceLoading = false
    spaceError = nil
    spaceWebView.loadHTMLString("", baseURL: nil)
  }
}
