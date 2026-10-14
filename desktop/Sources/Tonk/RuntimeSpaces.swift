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
            name: {...field('xyz.tonk.space/name', 'Text'), optional: true}
          }},
          terms: {this: variable('this'), subject: variable('subject'), name: variable('name')}
        };
        const rows = await api(path, query);
        \(RuntimeCatalogObservation.script)
        if (!Array.isArray(rows)) throw new Error('Invalid space catalog response.');
        return {branch, root: identity.rootDid || '', status: account.status, spaces: rows.map(row => ({subject: row.fields.subject, name: row.fields.name || null}))};
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

  func openSpace(_ space: TonkSpace) {
    loading = true
    ready = false
    selectedSpace = space
    error = nil
    webView.load(URLRequest(url: space.url))
  }

  func showSpaces() {
    selectedSpace = nil
    Task { await refreshSpaces() }
  }
}
