import AppKit
import Foundation
import HarnessCore

/// Links, people and views for each space, read through the signed-in
/// runtime. Every request goes through the worker, which enforces authority.
@MainActor
final class AccessModel: ObservableObject {
  @Published private(set) var spaces: [String: SpaceAccess] = [:]
  @Published private(set) var loading: Set<String> = []
  @Published private(set) var errors: [String: String] = [:]
  @Published private(set) var agentConnections: [AgentConnection] = []
  @Published private(set) var accountName: String?
  @Published private(set) var selfDid: String?
  @Published private(set) var journal = AccessJournal()
  /// Link operations in flight, keyed by link id or `new:<subject>`.
  @Published private(set) var working: Set<String> = []
  @Published var notice: String?

  weak var runtime: RuntimeModel?
  private var journalURL: URL?
  private let vault = LinkVault(scope: RuntimeLocation.deployment.title)
  private var accountLoaded = false
  /// The account's spaces, in catalog order; counts and pages follow it.
  private var catalog: [String] = []
  /// Spaces asked to refresh while a refresh was already reading them.
  private var refreshAgain: Set<String> = []
  /// Bumped by `reset`, so reads begun under another account are dropped.
  private var generation = 0

  /// For layout fixtures only: show prepared access without a runtime.
  func showFixture(
    _ fixture: [String: SpaceAccess], agents: [AgentConnection], accountName: String,
    journal: AccessJournal
  ) {
    spaces = fixture
    catalog = fixture.keys.sorted()
    agentConnections = agents
    self.accountName = accountName
    self.journal = journal
  }

  func prepare(directory: URL, runtime: RuntimeModel) {
    self.runtime = runtime
    journalURL = directory.appendingPathComponent("access-journal.json")
    if let journalURL { journal = AccessJournal.load(from: journalURL) }
  }

  /// Clears everything learned under the previous account.
  func reset() {
    generation += 1
    catalog = []
    refreshAgain = []
    spaces = [:]
    errors = [:]
    agentConnections = []
    accountName = nil
    selfDid = nil
    accountLoaded = false
  }

  // MARK: Reads

  func links(for subject: String) -> [AccessLink] { spaces[subject]?.links ?? [] }

  var allLinks: [AccessLink] { catalog.flatMap { links(for: $0) } }

  /// Whether every catalog space has been read, successfully or not.
  var loadedAll: Bool { catalog.allSatisfy { spaces[$0] != nil || errors[$0] != nil } }

  /// Follow the account's catalog, dropping spaces that left it.
  func setCatalog(_ spaces: [TonkSpace]) {
    catalog = spaces.map(\.id)
    let keep = Set(catalog)
    self.spaces = self.spaces.filter { keep.contains($0.key) }
    errors = errors.filter { keep.contains($0.key) || $0.key == "account" }
  }

  var agentLinks: [AccessLink] { allLinks.filter(\.forAgents) }

  func madeAt(_ link: AccessLink) -> Date? { journal.madeAt(target: link.target) }

  /// A link URL this Mac minted and still holds.
  func storedLink(_ link: AccessLink) -> String? {
    if link.kind == .publicRead { return spaces[link.subject]?.publicAddress }
    return vault.read(link.target)
  }

  func refreshAccount() async {
    guard let runtime else { return }
    let started = generation
    do {
      let result = try await runtime.accountScript(Self.accountScript)
      guard generation == started else { return }
      let data = try JSONSerialization.data(withJSONObject: result["connections"] ?? [])
      agentConnections = try JSONDecoder().decode([AgentConnection].self, from: data)
      accountName = (result["name"] as? String).flatMap { $0.isEmpty ? nil : $0 }
      selfDid = (result["root"] as? String).flatMap { $0.isEmpty ? nil : $0 }
      accountLoaded = true
    } catch {
      if generation == started { errors["account"] = Self.message(error) }
    }
  }

  static let accountScript = """
    const [profile, root] = await Promise.all([api('/api/profile'), api('/api/identity/root')]);
    // Agent links exist only on workers built with connection invites.
    const response = await fetch('/api/account/connections', {signal: AbortSignal.timeout(60000)});
    const connections = response.ok ? await response.json() : [];
    return {name: profile.display_name || profile.displayName || '', root: root.rootDid || '',
            connections: Array.isArray(connections) ? connections : []};
    """

  /// Read a space. A call while one is running makes it read again afterwards,
  /// so the result always reflects every action that preceded the call.
  func refresh(_ space: TonkSpace) async {
    guard runtime != nil else { return }
    if loading.contains(space.id) {
      refreshAgain.insert(space.id)
      return
    }
    loading.insert(space.id)
    defer { loading.remove(space.id) }
    repeat {
      refreshAgain.remove(space.id)
      await read(space)
    } while refreshAgain.contains(space.id)
  }

  private func read(_ space: TonkSpace) async {
    guard let runtime else { return }
    let started = generation
    if !accountLoaded { await refreshAccount() }
    do {
      let result = try await runtime.accountScript(
        Self.spaceScript, arguments: ["subject": space.subject])
      guard generation == started else { return }
      let invites = try Self.decode([InviteSummary].self, result["invites"])
      let members = Self.members(result["members"])
      let isPublic = (result["executions"] as? [[String: Any]] ?? []).contains {
        ($0["fields"] as? [String: Any])?["kind"] as? String == "public"
      }
      var access = SpaceAccess(
        members: members,
        links: SpaceAccess.links(
          subject: space.subject, invites: invites, agents: agentConnections, members: members,
          isPublic: isPublic),
        agents: agentConnections.filter { $0.subject == space.subject },
        views: Self.views(home: result["home"], directories: result["directories"]),
        isPublic: isPublic, selfDid: selfDid)
      access.publicAddress = Self.publicAddress(result["remote"], subject: space.subject)
      spaces[space.id] = access
      errors[space.id] = nil
    } catch {
      if generation == started { errors[space.id] = Self.message(error) }
    }
  }

  /// Load each space once, sequentially, so counts fill in without a burst.
  func refreshAll(_ catalog: [TonkSpace]) async {
    setCatalog(catalog)
    await refreshAccount()
    for space in catalog {
      guard !Task.isCancelled else { return }
      await refresh(space)
    }
  }

  /// One read per space: invites, roster, publications, views and remote.
  static let spaceScript = """
    const base = '/api/repository/' + encodeURIComponent(subject);
    const main = base + '/branch/main';
    const variable = name => ({'?': {name}});
    const field = (the, as, optional) => ({the, as, cardinality: 'one', ...(optional ? {optional: true} : {})});
    const query = (with_, names) => api(main + '/query', {
      predicate: {with: with_},
      terms: Object.fromEntries(['this', ...names].map(name => [name, variable(name)]))
    });
    const notation = async document => {
      const response = await fetch(main + '/evaluate?transact=false', {
        method: 'POST', headers: {'content-type': 'text/plain'}, body: document,
        signal: AbortSignal.timeout(60000)
      });
      if (!response.ok) return [];
      const value = await response.json();
      return (value.matches_after || value.matchesAfter || []).flatMap(block => block.results || []);
    };
    // A missing part leaves its section empty instead of failing the overview.
    // The roster is required: an empty one would wrongly read as "only you".
    const safe = (promise, fallback) => promise.catch(() => fallback);
    const [invites, members, executions, home, directories, concepts, info] = await Promise.all([
      safe(api(base + '/invites'), []),
      query({
        member: field('xyz.tonk.membership/member', 'Entity'),
        role: field('xyz.tonk.membership/role', 'Entity', true),
        name: field('xyz.tonk.membership/name', 'Text', true),
        invitation: field('xyz.tonk.membership/invitation', 'Entity', true)
      }, ['member', 'role', 'name', 'invitation']),
      safe(query({
        subject: field('xyz.tonk.invitation/subject', 'Entity'),
        kind: field('xyz.tonk.invitation-execution/kind', 'Text')
      }, ['subject', 'kind']), []),
      safe(notation('name:\\n  this: id:tonk/space\\n'), []),
      safe(query({directory: field('xyz.tonk.view/directory', 'Text')}, ['directory']), []),
      safe(notation('concept:\\n'), []),
      safe(api(base), {})
    ]);
    // A concept is the space's own when one of its attributes is outside the
    // namespaces the shipped library uses; only those directories are views.
    const reserved = /^(xyz\\.tonk|dialog|tonk|db)[.\\/:]/;
    const described = Object.fromEntries(concepts.map(row => {
      let source = row.fields?.source;
      try { if (typeof source === 'string') source = JSON.parse(source); } catch { source = null; }
      const attributes = Object.values(source?.with || {})
        .map(field => typeof field?.the === 'string' ? field.the : field?.the?.domain || '');
      return [row.this, {name: typeof row.fields?.name === 'string' ? row.fields.name : null,
        own: attributes.some(the => the && !reserved.test(the))}];
    }));
    const homeModel = home[0]?.fields?.entity || null;
    const models = directories.map(row => row.this).filter(x => typeof x === 'string');
    return {
      invites, members, executions,
      home: homeModel ? {model: homeModel, name: described[homeModel]?.name || null} : null,
      directories: [...new Set(models)].filter(model => described[model]?.own)
        .map(model => ({model, name: described[model].name})),
      remote: Object.values(info.remote || {}).map(r => r?.address?.Ucan?.endpoint).find(x => typeof x === 'string') || null
    };
    """

  static func members(_ value: Any?) -> [SpaceMember] {
    let rows = value as? [[String: Any]] ?? []
    var seen = Set<String>()
    return rows.compactMap { row in
      guard let fields = row["fields"] as? [String: Any],
        let did = fields["member"] as? String, seen.insert(did).inserted
      else { return nil }
      return SpaceMember(
        did: did, name: fields["name"] as? String, role: fields["role"] as? String,
        invitation: fields["invitation"] as? String)
    }
    .sorted { ($0.isFounder ? 0 : 1, $0.displayName) < ($1.isFounder ? 0 : 1, $1.displayName) }
  }

  /// The home first, then directories over the space's own concepts, named
  /// by their concept names rather than their content-derived URIs.
  static func views(home: Any?, directories: Any?) -> [SpaceViewEntry] {
    func entry(_ value: Any?, isHome: Bool) -> SpaceViewEntry? {
      guard let row = value as? [String: Any], let model = row["model"] as? String else {
        return nil
      }
      let name = (row["name"] as? String).flatMap { $0.isEmpty ? nil : $0 }
      return SpaceViewEntry(
        model: model, title: SpaceViewEntry.title(forModel: name ?? model), isHome: isHome)
    }
    var entries: [SpaceViewEntry] = []
    if let home = entry(home, isHome: true), home.model != "tonk:blank" {
      entries.append(home)
    } else {
      entries.append(SpaceViewEntry(model: "tonk/space", title: "Home", isHome: true))
    }
    for view in (directories as? [Any] ?? []).compactMap({ entry($0, isHome: false) })
    where !entries.contains(where: { $0.model == view.model }) {
      entries.append(view)
    }
    return entries.prefix(1) + entries.dropFirst().sorted { $0.title < $1.title }
  }

  static func publicAddress(_ endpoint: Any?, subject: String) -> String? {
    guard let endpoint = endpoint as? String, let base = URL(string: endpoint),
      ["https", "http"].contains(base.scheme), base.host != nil,
      let url = URL(string: "/space/" + subject, relativeTo: base)
    else { return nil }
    return url.absoluteString
  }

  // MARK: Actions

  enum NewLink: String, CaseIterable, Identifiable {
    case person, agent, publicRead
    var id: String { rawValue }
  }

  /// Mints a link, keeps it for Copy, copies it and records it.
  @discardableResult
  func make(_ kind: NewLink, in space: TonkSpace) async -> Bool {
    guard let runtime, !working.contains("new:" + space.id) else { return false }
    working.insert("new:" + space.id)
    defer { working.remove("new:" + space.id) }
    do {
      switch kind {
      case .person:
        let (url, target) = try await mintPersonLink(runtime: runtime, space: space)
        finishMint(url: url, target: target, kind: .person, space: space)
      case .agent:
        let before = Set(agentConnections.map(\.id))
        let link = try await runtime.createCLILink(space)
        await refreshAccount()
        let fresh = agentConnections.filter {
          $0.subject == space.subject && !before.contains($0.id)
        }
        finishMint(
          url: link, target: fresh.count == 1 ? fresh[0].id : nil, kind: .agent, space: space)
      case .publicRead:
        try await setPublic(true, space: space, runtime: runtime)
        await refresh(space)
        record(.published, kind: .publicRead, target: space.subject, space: space)
        if let address = spaces[space.id]?.publicAddress, copy(address) {
          notice = "Public link copied"
        } else {
          notice = "The space is public"
        }
        return true
      }
      await refresh(space)
      return true
    } catch {
      notice = "Couldn’t make the link: " + Self.message(error)
      return false
    }
  }

  func copyLink(_ link: AccessLink) {
    guard let value = storedLink(link) else { return }
    notice = copy(value) ? "Link copied" : "Couldn’t copy the link"
  }

  func revoke(_ link: AccessLink, space: TonkSpace) async {
    guard let runtime, !working.contains(link.id) else { return }
    working.insert(link.id)
    defer { working.remove(link.id) }
    do {
      try await revoke(link, runtime: runtime, space: space)
      record(
        link.kind == .publicRead ? .unpublished : .revoked, kind: link.kind, target: link.target,
        space: space)
      if link.kind != .publicRead { vault.remove(link.target) }
      notice = link.kind == .publicRead ? "The space is private" : "Link revoked"
      if link.kind == .agent { await refreshAccount() }
      await refresh(space)
    } catch {
      notice = "Couldn’t revoke the link: " + Self.message(error)
    }
  }

  /// Replace a plain link: the new link works before the old one stops.
  func resetLink(_ link: AccessLink, space: TonkSpace) async {
    guard let runtime, link.canReset, !working.contains(link.id) else { return }
    working.insert(link.id)
    defer { working.remove(link.id) }
    do {
      let (url, target) = try await mintPersonLink(runtime: runtime, space: space)
      if let target { try? vault.write(url, for: target) }
      _ = copy(url)
      do {
        try await revoke(link, runtime: runtime, space: space)
        vault.remove(link.target)
        record(.reset, kind: .person, target: target, space: space)
        notice = "New link copied. The old link no longer works."
      } catch {
        record(.made, kind: .person, target: target, space: space)
        notice = "New link copied, but the old link is still active: " + Self.message(error)
      }
      await refresh(space)
    } catch {
      notice = "Couldn’t reset the link: " + Self.message(error)
    }
  }

  private func mintPersonLink(runtime: RuntimeModel, space: TonkSpace) async throws
    -> (String, String?)
  {
    let result = try await runtime.accountScript(
      """
      const base = '/api/repository/' + encodeURIComponent(subject);
      const before = new Set((await api(base + '/invites')).map(row => row.targetCid));
      const minted = await api(base + '/invite', {});
      const after = await api(base + '/invites');
      return {kind: minted.kind, url: minted.url,
              targets: after.map(row => row.targetCid).filter(cid => !before.has(cid))};
      """, arguments: ["subject": space.subject])
    // Ticket links live on the host serving the space, the public address's origin.
    var origins = [RuntimeLocation.home]
    if let address = spaces[space.id]?.publicAddress, let host = URL(string: address) {
      origins.append(host)
    }
    let url = try RuntimeModel.validateShareLink(
      result, subject: space.subject, origins: origins)
    let targets = result["targets"] as? [String] ?? []
    return (url.absoluteString, targets.count == 1 ? targets[0] : nil)
  }

  private func finishMint(url: String, target: String?, kind: AccessLinkKind, space: TonkSpace) {
    if let target { try? vault.write(url, for: target) }
    record(.made, kind: kind, target: target, space: space)
    notice =
      copy(url)
      ? (target == nil
        ? "Link copied. Save it now; this Mac couldn’t match it to its listing."
        : "Link copied")
      : "The link was made, but couldn’t be copied"
  }

  private func revoke(_ link: AccessLink, runtime: RuntimeModel, space: TonkSpace) async throws {
    switch link.kind {
    case .person, .scoped:
      _ = try await runtime.accountScript(
        "return await api('/api/repository/' + encodeURIComponent(subject) + '/invites/' + encodeURIComponent(target) + '/revoke', {});",
        arguments: ["subject": space.subject, "target": link.target])
    case .agent:
      let result = try await runtime.accountScript(
        "return await api('/api/account/connections/' + encodeURIComponent(target) + '/revoke', {});",
        arguments: ["target": link.target])
      guard result["status"] as? String == "revoked" else {
        throw HarnessError.message("Revocation is not confirmed yet. Refresh to check again.")
      }
    case .publicRead:
      try await setPublic(false, space: space, runtime: runtime)
    }
  }

  /// Publication is a profile command with no receipt; wait for its record.
  private func setPublic(_ published: Bool, space: TonkSpace, runtime: RuntimeModel) async throws {
    guard let branch = runtime.catalogBranch else {
      throw HarnessError.message("Wait for your spaces to load, then try again.")
    }
    let domain = published ? "xyz.tonk.publish" : "xyz.tonk.unpublish"
    let result = try await runtime.accountScript(
      """
      const field = (the, as) => ({the, as, cardinality: 'one'});
      await api('/api/profile/branch/' + encodeURIComponent(branch) + '/transact', {claims: [{op: 'assert', application: {
        predicate: {kind: 'transient', concept: {with: {
          space: field(domain + '/space', 'Entity'), time: field(domain + '/time', 'Float')
        }}}, parameters: {space: subject, time: Date.now()}
      }}]});
      const main = '/api/repository/' + encodeURIComponent(subject) + '/branch/main/query';
      for (let attempt = 0; attempt < 30; attempt++) {
        await new Promise(resolve => setTimeout(resolve, 1000));
        const rows = await api(main, {
          predicate: {with: {
            subject: field('xyz.tonk.invitation/subject', 'Entity'),
            kind: field('xyz.tonk.invitation-execution/kind', 'Text')
          }},
          terms: {this: {'?': {name: 'this'}}, subject: {'?': {name: 'subject'}}, kind: {'?': {name: 'kind'}}}
        });
        const live = rows.some(row => row.fields?.kind === 'public');
        if (live === published) return {done: true};
      }
      return {done: false};
      """,
      arguments: [
        "subject": space.subject, "branch": branch, "domain": domain, "published": published,
      ])
    guard result["done"] as? Bool == true else {
      throw HarnessError.message(
        published
          ? "The space did not confirm it is public. Only admins can publish a space."
          : "The space did not confirm it is private. Only admins can unpublish a space.")
    }
  }

  private func record(
    _ action: AccessEvent.Action, kind: AccessLinkKind, target: String?, space: TonkSpace
  ) {
    journal.record(
      AccessEvent(date: Date(), subject: space.subject, action: action, kind: kind, target: target))
    if let journalURL { try? journal.save(to: journalURL) }
  }

  private func copy(_ value: String) -> Bool {
    NSPasteboard.general.clearContents()
    return NSPasteboard.general.setString(value, forType: .string)
  }

  private static func decode<T: Decodable>(_ type: T.Type, _ value: Any?) throws -> T {
    try JSONDecoder().decode(type, from: JSONSerialization.data(withJSONObject: value ?? []))
  }

  static func message(_ error: Error) -> String {
    (error as NSError).userInfo["WKJavaScriptExceptionMessage"] as? String
      ?? error.localizedDescription
  }
}
