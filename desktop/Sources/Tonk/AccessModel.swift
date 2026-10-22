import AppKit
import Foundation
import HarnessCore

@MainActor
protocol SpaceAccessRuntime: AnyObject {
  var catalogBranch: String? { get }
  func accountScript(_ script: String, arguments: [String: Any]) async throws -> [String: Any]
  func createCLILink(_ space: TonkSpace) async throws -> String
}

extension SpaceAccessRuntime {
  func accountScript(_ script: String) async throws -> [String: Any] {
    try await accountScript(script, arguments: [:])
  }
}

extension RuntimeModel: SpaceAccessRuntime {}

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
  @Published private(set) var accountEmail: String?
  @Published private(set) var accountPasskeys: [AccountPasskey] = []
  @Published private(set) var accountDetailsLoaded = false
  @Published private(set) var accountDetailsError: String?
  @Published private(set) var passkeyCreatedOn: String?
  @Published private(set) var passkeyCreatedAt: Date?
  @Published private(set) var journal = AccessJournal()
  /// Link operations in flight, keyed by link id or `new:<subject>`.
  @Published private(set) var working: Set<String> = []
  @Published var notice: String?

  @Published private(set) var savedContacts: [String: [DesktopContact]] = [:]
  private var contactsURL: URL?

  var contacts: [DesktopContact] {
    var byID = Dictionary(
      (savedContacts[selfDid ?? ""] ?? []).map { ($0.id, $0) },
      uniquingKeysWith: { first, _ in first })
    for state in spaces.values {
      for member in state.members where byID[member.did] == nil {
        byID[member.did] = DesktopContact(id: member.did, name: member.displayName, labels: [])
      }
    }
    return byID.values.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
  }

  func saveContact(_ contact: DesktopContact) throws {
    guard let root = selfDid, let contactsURL else {
      throw HarnessError.message("Sign in before saving a contact.")
    }
    var updated = savedContacts
    var list = updated[root] ?? []
    list.removeAll { $0.id == contact.id }
    list.append(contact)
    updated[root] = list
    try JSONEncoder().encode(updated).write(to: contactsURL, options: .atomic)
    savedContacts = updated
  }

  func updateDisplayName(_ name: String) async throws {
    guard let runtime else { throw HarnessError.message("Sign in before changing your name.") }
    let started = generation
    let result = try await runtime.accountScript(
      "return await api('/api/account/display-name', {name});", arguments: ["name": name])
    guard generation == started else { return }
    accountName = result["name"] as? String ?? name
  }

  weak var runtime: (any SpaceAccessRuntime)?
  private var journalURL: URL?
  private let vault: any AccessLinkStore
  private let copyToPasteboard: (String) -> Bool
  private var accountLoaded = false
  /// The account's spaces, in catalog order; counts and pages follow it.
  private var catalog: [String] = []
  /// Spaces asked to refresh while a refresh was already reading them.
  private var refreshAgain: Set<String> = []
  /// Bumped by `reset`, so reads begun under another account are dropped.
  private var generation = 0

  init(
    vault: any AccessLinkStore = LinkVault(scope: RuntimeLocation.deployment.title),
    copyToPasteboard: @escaping (String) -> Bool = {
      NSPasteboard.general.clearContents()
      return NSPasteboard.general.setString($0, forType: .string)
    }
  ) {
    self.vault = vault
    self.copyToPasteboard = copyToPasteboard
  }

  /// For layout fixtures only: show prepared access without a runtime.
  func showFixture(
    _ fixture: [String: SpaceAccess], agents: [AgentConnection], accountName: String,
    journal: AccessJournal, email: String? = nil, passkeys: [AccountPasskey] = []
  ) {
    spaces = fixture
    catalog = fixture.keys.sorted()
    agentConnections = agents
    self.accountName = accountName
    self.accountEmail = email
    self.accountPasskeys = passkeys
    self.accountDetailsLoaded = email != nil || !passkeys.isEmpty
    self.journal = journal
  }

  func prepare(directory: URL, runtime: any SpaceAccessRuntime) {
    self.runtime = runtime
    contactsURL = directory.appendingPathComponent("address-book.json")
    if let contactsURL, let data = try? Data(contentsOf: contactsURL) {
      savedContacts =
        (try? JSONDecoder().decode([String: [DesktopContact]].self, from: data)) ?? [:]
    }
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
    passkeyCreatedOn = nil
    passkeyCreatedAt = nil
    accountEmail = nil
    accountPasskeys = []
    accountDetailsLoaded = false
    accountDetailsError = nil
    accountLoaded = false
    loading = []
    working = []
    notice = nil
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

  /// Live grants something has connected through; unused links stay in All links.
  var connections: [AgentConnection] { agentConnections.filter { $0.isLive && $0.hasConnected } }

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
      let passkey = result["passkey"] as? [String: Any]
      passkeyCreatedOn = passkey?["createdOn"] as? String
      passkeyCreatedAt = (passkey?["createdAt"] as? Double).map(Date.init(timeIntervalSince1970:))
      accountEmail = result["email"] as? String
      accountDetailsLoaded = result["accountDetailsLoaded"] as? Bool ?? false
      accountDetailsError = result["accountDetailsError"] as? String
      accountPasskeys = (result["passkeys"] as? [[String: Any]] ?? []).compactMap { row in
        guard let id = row["id"] as? String, let label = row["createdOn"] as? String else {
          return nil
        }
        return AccountPasskey(
          id: id, createdOn: label,
          createdAt: (row["createdAt"] as? Double).map(Date.init(timeIntervalSince1970:)))
      }
      accountLoaded = true
      errors["account"] = nil
    } catch {
      if generation == started { errors["account"] = Self.message(error) }
    }
  }

  static let accountScript = """
    const [profile, root] = await Promise.all([api('/api/profile'), api('/api/identity/root')]);
    // Agent links exist only on workers built with connection invites.
    const response = await fetch('/api/account/connections', {signal: AbortSignal.timeout(60000)});
    if (!response.ok) throw new Error('Agent connections could not be loaded (HTTP ' + response.status + ').');
    const connections = await response.json();
    if (!Array.isArray(connections)) throw new Error('Invalid agent connections response.');
    let details = {};
    if (root.rootDid) {
      try {
        details = {...await (async () => { \(AccountDisplayFacts.script) })(), accountDetailsLoaded: true};
      } catch { details = {accountDetailsError: 'Account details could not be loaded. Try refreshing.'}; }
    }
    return {...details, name: profile.display_name || profile.displayName || '', root: root.rootDid || '', passkey: root.passkey || null,
            connections: Array.isArray(connections) ? connections : []};
    """

  /// Read a space. A call while one is running makes it read again afterwards,
  /// so the result always reflects every action that preceded the call.
  func refresh(_ space: TonkSpace) async {
    guard runtime != nil else { return }
    let started = generation
    if loading.contains(space.id) {
      refreshAgain.insert(space.id)
      return
    }
    loading.insert(space.id)
    defer { if generation == started { loading.remove(space.id) } }
    repeat {
      refreshAgain.remove(space.id)
      await read(space)
    } while generation == started && refreshAgain.contains(space.id)
  }

  private func read(_ space: TonkSpace) async {
    guard let runtime else { return }
    let started = generation
    if !accountLoaded { await refreshAccount() }
    guard generation == started else { return }
    do {
      let result = try await runtime.accountScript(
        Self.spaceScript, arguments: ["subject": space.subject])
      guard generation == started else { return }
      let revoked = journal.revokedTargets[space.subject] ?? []
      let invites = try Self.decode([InviteSummary].self, result["invites"])
        .filter { !revoked.contains($0.targetCid) }
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
      let warnings = result["warnings"] as? [String] ?? []
      errors[space.id] = warnings.isEmpty ? nil : warnings.joined(separator: " ")
    } catch {
      if generation == started { errors[space.id] = Self.message(error) }
    }
  }

  /// Load each space once, sequentially, so counts fill in without a burst.
  func refreshAll(_ catalog: [TonkSpace]) async {
    let started = generation
    setCatalog(catalog)
    await refreshAccount()
    for space in catalog {
      guard !Task.isCancelled, generation == started else { return }
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
      if (!response.ok) throw new Error('View query failed (HTTP ' + response.status + ').');
      const value = await response.json();
      return (value.matches_after || value.matchesAfter || []).flatMap(block => block.results || []);
    };
    // Keep partial data useful, but never present failed reads as a verified
    // absence of links, publications, or views.
    const warnings = [];
    const safe = (promise, fallback, section) => promise.catch(() => {
      warnings.push(section + ' could not be loaded. Try refreshing.');
      return fallback;
    });
    const [invites, members, executions, home, directories, concepts, info] = await Promise.all([
      safe(api(base + '/invites'), [], 'Links'),
      query({
        member: field('xyz.tonk.membership/member', 'Entity'),
        role: field('xyz.tonk.membership/role', 'Entity', true),
        name: field('xyz.tonk.membership/name', 'Text', true),
        invitation: field('xyz.tonk.membership/invitation', 'Entity', true)
      }, ['member', 'role', 'name', 'invitation']),
      safe(query({
        subject: field('xyz.tonk.invitation/subject', 'Entity'),
        kind: field('xyz.tonk.invitation-execution/kind', 'Text')
      }, ['subject', 'kind']), [], 'Public access'),
      safe(notation('name:\\n  this: id:tonk/space\\n'), [], 'Home view'),
      safe(query({directory: field('xyz.tonk.view/directory', 'Text')}, ['directory']), [], 'Views'),
      safe(notation('concept:\\n'), [], 'View names'),
      safe(api(base), {}, 'Space address')
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
      invites, members, executions, warnings,
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
    guard runtime != nil, !working.contains("new:" + space.id) else { return false }
    let (journey, action): (DesktopAnalytics.Journey, DesktopAnalytics.Action) =
      switch kind {
      case .person: (.collaboration, .copyShareLink)
      case .agent: (.handoff, .connectAgent)
      case .publicRead: (.space, .publishSpace)
      }
    let attempt = Telemetry.shared.begin(journey, action, surface: .hub)
    let started = generation
    let made = await mint(kind, in: space)
    if made {
      attempt.finish(.success, stage: kind == .publicRead ? .remoteCommit : .clipboard)
    } else {
      attempt.finish(generation == started ? .retryableFailure : .cancelled, stage: .worker)
    }
    return made
  }

  private func mint(_ kind: NewLink, in space: TonkSpace) async -> Bool {
    guard let runtime, !working.contains("new:" + space.id) else { return false }
    let started = generation
    working.insert("new:" + space.id)
    defer { if generation == started { working.remove("new:" + space.id) } }
    do {
      var copied = false
      switch kind {
      case .person:
        let (url, target) = try await mintPersonLink(runtime: runtime, space: space)
        guard generation == started else { return false }
        copied = finishMint(url: url, target: target, kind: .person, space: space)
      case .agent:
        let before = Set(agentConnections.map(\.id))
        let link = try await runtime.createCLILink(space)
        guard generation == started else { return false }
        await refreshAccount()
        guard generation == started else { return false }
        let fresh = agentConnections.filter {
          $0.subject == space.subject && !before.contains($0.id)
        }
        copied = finishMint(
          url: link, target: fresh.count == 1 ? fresh[0].id : nil, kind: .agent, space: space)
      case .publicRead:
        try await setPublic(true, space: space, runtime: runtime)
        guard generation == started else { return false }
        await refresh(space)
        guard generation == started else { return false }
        record(.published, kind: .publicRead, target: space.subject, space: space)
        if let address = spaces[space.id]?.publicAddress, copy(address) {
          notice = "Public link copied"
        } else {
          notice = "The space is public"
        }
        return true
      }
      await refresh(space)
      return copied
    } catch {
      guard generation == started else { return false }
      notice = "Couldn’t make the link: " + Self.message(error)
      return false
    }
  }

  /// Reuse an existing open grant; never mint another merely because this Mac
  /// cannot recover its secret. That case requires the explicit Reset action.
  func defaultLink(in space: TonkSpace) -> AccessLink? {
    let links = links(for: space.id).filter { $0.kind == .person && $0.status == "active" }
    return links.first(where: { storedLink($0) != nil }) ?? links.first
  }

  @discardableResult
  func copyDefaultLink(in space: TonkSpace) async -> Bool {
    if spaces[space.id] == nil { await refresh(space) }
    guard spaces[space.id] != nil, errors[space.id] == nil else {
      notice = "Refresh this space before copying its default link."
      return false
    }
    if let link = defaultLink(in: space) {
      guard let value = storedLink(link) else {
        notice =
          "This Mac does not have the default link. Reset it in Links to create a replacement."
        return false
      }
      let copied = copy(value)
      notice = copied ? "Link copied" : "Couldn’t copy the link"
      return copied
    }
    return await make(.person, in: space)
  }

  func copyLink(_ link: AccessLink) {
    guard let value = storedLink(link) else { return }
    notice = copy(value) ? "Link copied" : "Couldn’t copy the link"
  }

  func revoke(_ link: AccessLink, space: TonkSpace) async {
    guard runtime != nil, !working.contains(link.id) else { return }
    let attempt = Telemetry.shared.begin(
      link.kind == .publicRead ? .space : .collaboration,
      link.kind == .publicRead ? .unpublishSpace : .revokeLink, surface: .hub)
    let started = generation
    let revoked = await withdraw(link, space: space)
    if revoked {
      attempt.finish(.success, stage: .remoteCommit)
    } else {
      attempt.finish(generation == started ? .retryableFailure : .cancelled, stage: .worker)
    }
  }

  private func withdraw(_ link: AccessLink, space: TonkSpace) async -> Bool {
    guard let runtime, !working.contains(link.id) else { return false }
    let started = generation
    working.insert(link.id)
    defer { if generation == started { working.remove(link.id) } }
    do {
      try await revoke(link, runtime: runtime, space: space)
      guard generation == started else { return false }
      record(
        link.kind == .publicRead ? .unpublished : .revoked, kind: link.kind, target: link.target,
        space: space)
      if link.kind != .publicRead { vault.remove(link.target) }
      notice = link.kind == .publicRead ? "The space is private" : "Link revoked"
      if link.kind == .agent { await refreshAccount() }
      guard generation == started else { return false }
      await refresh(space)
      return true
    } catch {
      guard generation == started else { return false }
      notice = "Couldn’t revoke the link: " + Self.message(error)
      return false
    }
  }

  /// Replace a plain link: the new link works before the old one stops.
  func resetLink(_ link: AccessLink, space: TonkSpace) async {
    guard let runtime, link.canReset, !working.contains(link.id) else { return }
    let started = generation
    working.insert(link.id)
    defer { if generation == started { working.remove(link.id) } }
    do {
      let (url, target) = try await mintPersonLink(runtime: runtime, space: space)
      guard generation == started else { return }
      let storageWarning = retain(url, target: target)
      guard copy(url) else {
        record(.made, kind: .person, target: target, space: space)
        notice =
          "A replacement was made but couldn’t be copied; the old link is still active."
          + storageWarning
        await refresh(space)
        return
      }
      do {
        try await revoke(link, runtime: runtime, space: space)
        guard generation == started else { return }
        vault.remove(link.target)
        journal.confirmRevocation(subject: space.subject, target: link.target)
        record(.reset, kind: .person, target: target, space: space)
        notice = "New link copied. The old link no longer works." + storageWarning
      } catch {
        guard generation == started else { return }
        record(.made, kind: .person, target: target, space: space)
        notice =
          "New link copied, but the old link is still active: " + Self.message(error)
          + storageWarning
      }
      await refresh(space)
    } catch {
      guard generation == started else { return }
      notice = "Couldn’t reset the link: " + Self.message(error)
    }
  }

  private func mintPersonLink(runtime: any SpaceAccessRuntime, space: TonkSpace) async throws
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

  private func finishMint(url: String, target: String?, kind: AccessLinkKind, space: TonkSpace)
    -> Bool
  {
    let storageWarning = retain(url, target: target)
    record(.made, kind: kind, target: target, space: space)
    let copied = copy(url)
    notice =
      (copied ? "Link copied." : "The link was made, but couldn’t be copied.")
      + storageWarning
    return copied
  }

  private func retain(_ url: String, target: String?) -> String {
    guard let target else {
      return " Save it now; this Mac couldn’t match it to its listing."
    }
    do {
      try vault.write(url, for: target)
      return ""
    } catch {
      return " This Mac couldn’t save the link in Keychain; Copy will not be available later."
    }
  }

  private func revoke(_ link: AccessLink, runtime: any SpaceAccessRuntime, space: TonkSpace)
    async throws
  {
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
  private func setPublic(_ published: Bool, space: TonkSpace, runtime: any SpaceAccessRuntime)
    async throws
  {
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
    copyToPasteboard(value)
  }

  private static func decode<T: Decodable>(_ type: T.Type, _ value: Any?) throws -> T {
    try JSONDecoder().decode(type, from: JSONSerialization.data(withJSONObject: value ?? []))
  }

  static func message(_ error: Error) -> String {
    (error as NSError).userInfo["WKJavaScriptExceptionMessage"] as? String
      ?? error.localizedDescription
  }
}
