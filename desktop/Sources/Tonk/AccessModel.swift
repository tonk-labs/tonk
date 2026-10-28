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

  /// Saved on this Mac, rather than only seen in a space's roster.
  func isSaved(_ did: String) -> Bool {
    savedContacts[selfDid ?? ""]?.contains { $0.id == did } == true
  }

  func saveContact(_ contact: DesktopContact) throws {
    guard let root = selfDid, let contactsURL else {
      throw HarnessError.message("Sign in before saving a contact.")
    }
    guard contact.emailIdentity == nil || contact.emailIdentity?.rootDid == contact.id else {
      throw HarnessError.message("The email lookup does not match this contact's key.")
    }
    var updated = savedContacts
    var list = updated[root] ?? []
    list.removeAll { $0.id == contact.id }
    list.append(contact)
    updated[root] = list
    try JSONEncoder().encode(updated).write(to: contactsURL, options: .atomic)
    savedContacts = updated
  }

  /// Persist the delivery ID before dispatch so an uncertain send can be retried after restart.
  private struct EmailInvitationJob: Codable {
    let id: String
    let createdAt: Date
  }

  func sendEmailInvitation(email: String, space: TonkSpace) async throws {
    guard let root = selfDid, let runtime, let branch = runtime.catalogBranch,
      let contactsURL else { throw HarnessError.message("Sign in before sending an invitation.") }
    let started = generation
    let address = email.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    let file = contactsURL.deletingLastPathComponent().appendingPathComponent("email-invitations.json")
    var jobs: [String: EmailInvitationJob] = [:]
    if FileManager.default.fileExists(atPath: file.path) {
      // A damaged retry record must never silently mint another invitation.
      jobs = try JSONDecoder().decode([String: EmailInvitationJob].self, from: Data(contentsOf: file))
    }
    let key = [root, space.subject, address].joined(separator: "|")
    let job: EmailInvitationJob
    if let existing = jobs[key], Date().timeIntervalSince(existing.createdAt) < 7 * 86400 {
      job = existing
    } else {
      job = EmailInvitationJob(
        id: (UUID().uuidString + UUID().uuidString).replacingOccurrences(of: "-", with: "").lowercased(),
        createdAt: Date())
      jobs[key] = job
      try JSONEncoder().encode(jobs).write(to: file, options: .atomic)
    }
    let result = try await runtime.accountScript(
      Self.emailInvitationScript,
      arguments: ["branch": branch, "account": root, "space": space.subject,
                  "email": address, "id": job.id, "request": "urn:uuid:" + UUID().uuidString])
    guard generation == started, selfDid == root else {
      throw HarnessError.message("The selected account changed. Check the original account before retrying.")
    }
    guard result["state"] as? String == "sent" else {
      throw HarnessError.message(result["message"] as? String
        ?? "Delivery could not be confirmed. Retry the same invitation.")
    }
  }

  static let emailInvitationScript = """
    const field = (the, as) => ({the, as, cardinality: 'one'});
    const domain = 'xyz.tonk.send-email-invitation';
    const base = '/api/repository/profile:tonk/branch/' + encodeURIComponent(branch);
    await api(base + '/transact', {claims: [{op: 'assert', application: {
      predicate: {kind: 'transient', concept: {with: {
        id: field(domain + '/id', 'Text'), email: field(domain + '/email', 'Text'),
        space: field(domain + '/space', 'Entity'), account: field(domain + '/account', 'Entity')
      }}}, parameters: {this: request, id, email, space, account}
    }}]});
    for (let attempt = 0; attempt < 45; attempt++) {
      await new Promise(resolve => setTimeout(resolve, 1000));
      const rows = await api(base + '/query', {
        predicate: {with: {
          state: field('xyz.tonk.email-invitation-status/state', 'Text'),
          message: field('xyz.tonk.email-invitation-status/message', 'Text')
        }}, terms: {this: request, state: {'?': {name: 'state'}}, message: {'?': {name: 'message'}}}
      });
      const answer = rows.find(row => row.fields?.state && row.fields.state !== 'pending');
      if (answer) return answer.fields;
    }
    return {state: 'unknown', message: 'Delivery could not be confirmed. Retry the same invitation.'};
    """

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
  /// Who the account is, kept on this Mac so the name shows before the worker answers.
  private struct AccountSnapshot: Codable {
    let root: String
    let name: String?
    let email: String?
  }
  private var accountSnapshotURL: URL?
  /// What each space showed last time, so links and people appear before the
  /// worker answers. Link secrets stay in the Keychain vault, not here.
  private struct SpacesSnapshot: Codable {
    let root: String
    let spaces: [String: SpaceAccess]
  }
  private var spacesSnapshotURL: URL?
  /// A full refresh is under way, including over cached rows already shown.
  @Published private(set) var refreshingAll = false
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

  /// `expectedRoot` is the account the cached catalog belongs to; the saved name
  /// is shown only for that same account.
  func prepare(directory: URL, runtime: any SpaceAccessRuntime, expectedRoot: String? = nil) {
    self.runtime = runtime
    accountSnapshotURL = directory.appendingPathComponent("account-display.json")
    spacesSnapshotURL = directory.appendingPathComponent("space-access.json")
    if let expectedRoot, let spacesSnapshotURL,
      let data = try? Data(contentsOf: spacesSnapshotURL),
      let snapshot = try? JSONDecoder().decode(SpacesSnapshot.self, from: data),
      snapshot.root == expectedRoot
    {
      spaces = snapshot.spaces
    }
    if let expectedRoot, let accountSnapshotURL,
      let data = try? Data(contentsOf: accountSnapshotURL),
      let snapshot = try? JSONDecoder().decode(AccountSnapshot.self, from: data),
      snapshot.root == expectedRoot
    {
      selfDid = snapshot.root
      accountName = snapshot.name
      accountEmail = snapshot.email
    }
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
    if let accountSnapshotURL { try? FileManager.default.removeItem(at: accountSnapshotURL) }
    if let spacesSnapshotURL { try? FileManager.default.removeItem(at: spacesSnapshotURL) }
    refreshingAll = false
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
      let root = (result["root"] as? String).flatMap { $0.isEmpty ? nil : $0 }
      // Before the account branch first syncs the name can read empty; keep the
      // one already shown for this same account rather than blanking it.
      accountName =
        (result["name"] as? String).flatMap { $0.isEmpty ? nil : $0 }
        ?? (root != nil && root == selfDid ? accountName : nil)
      selfDid = root
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
      if let root, let accountSnapshotURL {
        try? JSONEncoder().encode(
          AccountSnapshot(root: root, name: accountName, email: accountEmail)
        ).write(to: accountSnapshotURL, options: .atomic)
      }
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
    return {...details,
            name: details.displayName || profile.display_name || profile.displayName || '', root: root.rootDid || '', passkey: root.passkey || null,
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
          isPublic: isPublic,
          names: Dictionary(
            contacts.map { ($0.id, $0.name) }, uniquingKeysWith: { first, _ in first })),
        agents: agentConnections.filter { $0.subject == space.subject },
        views: Self.views(home: result["home"], directories: result["directories"]),
        isPublic: isPublic, selfDid: selfDid)
      access.publicAddress = Self.publicAddress(result["remote"], subject: space.subject)
      spaces[space.id] = access
      saveSpacesSnapshot()
      let warnings = result["warnings"] as? [String] ?? []
      errors[space.id] = warnings.isEmpty ? nil : warnings.joined(separator: " ")
    } catch {
      if generation == started { errors[space.id] = Self.message(error) }
    }
  }

  private func saveSpacesSnapshot() {
    guard let root = selfDid, let spacesSnapshotURL else { return }
    try? JSONEncoder().encode(SpacesSnapshot(root: root, spaces: spaces))
      .write(to: spacesSnapshotURL, options: .atomic)
  }

  /// Load each space once, sequentially, so counts fill in without a burst.
  func refreshAll(_ catalog: [TonkSpace]) async {
    let started = generation
    refreshingAll = true
    defer { if generation == started { refreshingAll = false } }
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
      if (!response.ok) {
        let reason = '';
        try {
          const message = (await response.json())?.error?.message;
          if (typeof message === 'string' && message) reason = ': ' + message.slice(0, 2000);
        } catch {}
        throw new Error('evaluate failed (HTTP ' + response.status + ')' + reason);
      }
      const value = await response.json();
      return (value.matches_after || value.matchesAfter || []).flatMap(block => block.results || []);
    };
    // Keep partial data useful, but never present failed reads as a verified
    // absence of links, publications, or views. Keep the cause (HTTP status
    // and worker message, or the timeout) so a failure can be diagnosed.
    const warnings = [];
    const safe = (promise, fallback, section) => promise.catch(error => {
      const cause = String(error?.message || error || 'unknown error');
      warnings.push(section + ' could not be loaded (' + cause + '). Try refreshing.');
      return fallback;
    });
    const [invites, members, executions, home, directories, concepts, info] = await Promise.all([
      safe(api(base + '/invites'), [], 'Links'),
      query({
        member: field('xyz.tonk.membership/member', 'Entity'),
        role: field('xyz.tonk.membership/role', 'Entity', true),
        name: field('xyz.tonk.membership/name', 'Text', true),
        invitation: field('xyz.tonk.membership/invitation', 'Entity', true),
        inviter: field('xyz.tonk.membership/inviter', 'Entity', true),
        kind: field('xyz.tonk.membership/kind', 'Entity', true)
      }, ['member', 'role', 'name', 'invitation', 'inviter', 'kind']),
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
        invitation: fields["invitation"] as? String, inviter: fields["inviter"] as? String,
        kind: fields["kind"] as? String)
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
  func make(
    _ kind: NewLink, in space: TonkSpace, surface: DesktopAnalytics.Surface = .hub
  ) async -> Bool {
    guard runtime != nil, !working.contains("new:" + space.id) else { return false }
    let (journey, action): (DesktopAnalytics.Journey, DesktopAnalytics.Action) =
      switch kind {
      case .person: (.collaboration, .copyShareLink)
      case .agent: (.handoff, .connectAgent)
      case .publicRead: (.space, .publishSpace)
      }
    let attempt = Telemetry.shared.begin(journey, action, surface: surface)
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
  /// Copies the space's default link, making one first if it has none. One click is one
  /// `copy_share_link` attempt; its receipt is the clipboard.
  func copyDefaultLink(
    in space: TonkSpace, surface: DesktopAnalytics.Surface = .hub
  ) async -> Bool {
    let attempt = Telemetry.shared.begin(.collaboration, .copyShareLink, surface: surface)
    if spaces[space.id] == nil { await refresh(space) }
    guard spaces[space.id] != nil, errors[space.id] == nil else {
      notice = "Refresh this space before copying its default link."
      attempt.finish(.retryableFailure, stage: .worker, failureKind: .serviceUnavailable)
      return false
    }
    if let link = defaultLink(in: space) {
      guard let value = storedLink(link) else {
        notice =
          "This Mac does not have the default link. Reset it in Links to create a replacement."
        attempt.finish(.blocked, stage: .validation, failureKind: .localState)
        return false
      }
      let copied = copy(value)
      notice = copied ? "Link copied" : "Couldn’t copy the link"
      attempt.finish(
        copied ? .success : .retryableFailure, stage: .clipboard,
        failureKind: copied ? nil : .localState)
      return copied
    }
    guard runtime != nil, !working.contains("new:" + space.id) else {
      attempt.finish(.blocked, stage: .intent, failureKind: .conflict)
      return false
    }
    let started = generation
    let made = await mint(.person, in: space)
    if made {
      attempt.finish(.success, stage: .clipboard)
    } else {
      attempt.finish(generation == started ? .retryableFailure : .cancelled, stage: .worker)
    }
    return made
  }

  func copyLink(_ link: AccessLink, surface: DesktopAnalytics.Surface = .hub) {
    guard let value = storedLink(link) else { return }
    let attempt = Telemetry.shared.begin(.collaboration, .copyShareLink, surface: surface)
    let copied = copy(value)
    notice = copied ? "Link copied" : "Couldn’t copy the link"
    attempt.finish(
      copied ? .success : .retryableFailure, stage: .clipboard,
      failureKind: copied ? nil : .localState)
  }

  /// Mints a link only `contact`'s account can claim, keeps it for Copy, copies
  /// it and records it. The contact's key must be an account root: that is what
  /// an email lookup saves and what a roster lists, and a claim checks it.
  @discardableResult
  func inviteContact(
    _ contact: DesktopContact, to space: TonkSpace, surface: DesktopAnalytics.Surface = .hub
  ) async -> Bool {
    guard let runtime, !working.contains("new:" + space.id) else { return false }
    let attempt = Telemetry.shared.begin(.collaboration, .inviteContact, surface: surface)
    guard contact.id.hasPrefix("did:key:"), contact.id != selfDid else {
      notice = "Choose someone else’s account to invite."
      attempt.finish(.blocked, stage: .validation, failureKind: .invalidInput)
      return false
    }
    let started = generation
    working.insert("new:" + space.id)
    defer { if generation == started { working.remove("new:" + space.id) } }
    do {
      let (url, target) = try await mintPersonLink(
        runtime: runtime, space: space, recipient: contact.id)
      guard generation == started else {
        attempt.finish(.cancelled, stage: .worker)
        return false
      }
      let copied = finishMint(url: url, target: target, kind: .scoped, space: space)
      await refresh(space)
      attempt.finish(
        copied ? .success : .retryableFailure, stage: .clipboard,
        failureKind: copied ? nil : .localState)
      return copied
    } catch {
      guard generation == started else {
        attempt.finish(.cancelled, stage: .worker)
        return false
      }
      notice = "Couldn’t make the link: " + Self.message(error)
      attempt.finish(.retryableFailure, stage: .worker)
      return false
    }
  }

  func revoke(
    _ link: AccessLink, space: TonkSpace, surface: DesktopAnalytics.Surface = .hub
  ) async {
    guard runtime != nil, !working.contains(link.id) else { return }
    let attempt = Telemetry.shared.begin(
      link.kind == .publicRead ? .space : .collaboration,
      link.kind == .publicRead ? .unpublishSpace : .revokeLink, surface: surface)
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

  /// Remove a member from a space: the worker revokes the grant that admits
  /// them, whichever link they joined with, and retracts their roster row.
  /// The command has no receipt, so success is the row's absence.
  func removeMember(_ did: String, name: String, from space: TonkSpace) async -> Bool {
    let key = "member:" + did
    guard let runtime, !working.contains(key) else { return false }
    let started = generation
    working.insert(key)
    defer { if generation == started { working.remove(key) } }
    do {
      let result = try await runtime.accountScript(
        Self.expelScript,
        arguments: [
          "subject": space.subject, "member": did, "request": "urn:uuid:" + UUID().uuidString,
        ])
      guard generation == started else { return false }
      guard result["done"] as? Bool == true else {
        throw HarnessError.message(
          "The space did not confirm it. Only admins can remove people, and someone who joined before this version may need the link they used revoked instead."
        )
      }
      notice = "\(name) was removed from \(space.title)"
      await refresh(space)
      return true
    } catch {
      guard generation == started else { return false }
      notice = "Couldn’t remove \(name): " + Self.message(error)
      return false
    }
  }

  static let expelScript = """
    const field = (the, as) => ({the, as, cardinality: 'one'});
    const base = '/api/repository/' + encodeURIComponent(subject) + '/branch/main';
    await api(base + '/transact', {claims: [{op: 'assert', application: {
      predicate: {kind: 'transient', concept: {with: {
        member: field('xyz.tonk.command.expel-member/member', 'Entity')
      }}}, parameters: {this: request, member}
    }}]});
    for (let attempt = 0; attempt < 30; attempt++) {
      await new Promise(resolve => setTimeout(resolve, 1000));
      const rows = await api(base + '/query', {
        predicate: {with: {member: field('xyz.tonk.membership/member', 'Entity')}},
        terms: {this: {'?': {name: 'this'}}, member}
      });
      if (rows.length === 0) return {done: true};
    }
    return {done: false};
    """

  /// Replace a plain link: the new link works before the old one stops.
  /// Replace a plain link. Fully replaced is success; a new link copied while the old one
  /// still works is degraded; nothing copied is a failure.
  func resetLink(
    _ link: AccessLink, space: TonkSpace, surface: DesktopAnalytics.Surface = .hub
  ) async {
    guard runtime != nil, link.canReset, !working.contains(link.id) else { return }
    let attempt = Telemetry.shared.begin(.collaboration, .resetLink, surface: surface)
    let started = generation
    let outcome = await replace(link, space: space)
    switch outcome {
    case .replaced: attempt.finish(.success, stage: .remoteCommit)
    case .oldStillActive: attempt.finish(.degradedSuccess, stage: .clipboard)
    case .notCopied:
      attempt.finish(.retryableFailure, stage: .clipboard, failureKind: .localState)
    case .failed:
      attempt.finish(generation == started ? .retryableFailure : .cancelled, stage: .worker)
    }
  }

  private enum ResetOutcome { case replaced, oldStillActive, notCopied, failed }

  private func replace(_ link: AccessLink, space: TonkSpace) async -> ResetOutcome {
    guard let runtime, link.canReset, !working.contains(link.id) else { return .failed }
    let started = generation
    working.insert(link.id)
    defer { if generation == started { working.remove(link.id) } }
    do {
      let (url, target) = try await mintPersonLink(runtime: runtime, space: space)
      guard generation == started else { return .failed }
      let storageWarning = retain(url, target: target)
      guard copy(url) else {
        record(.made, kind: .person, target: target, space: space)
        notice =
          "A replacement was made but couldn’t be copied; the old link is still active."
          + storageWarning
        await refresh(space)
        return .notCopied
      }
      do {
        try await revoke(link, runtime: runtime, space: space)
        guard generation == started else { return .failed }
        vault.remove(link.target)
        journal.confirmRevocation(subject: space.subject, target: link.target)
        record(.reset, kind: .person, target: target, space: space)
        notice = "New link copied. The old link no longer works." + storageWarning
        await refresh(space)
        return .replaced
      } catch {
        guard generation == started else { return .failed }
        record(.made, kind: .person, target: target, space: space)
        notice =
          "New link copied, but the old link is still active: " + Self.message(error)
          + storageWarning
        await refresh(space)
        return .oldStillActive
      }
    } catch {
      guard generation == started else { return .failed }
      notice = "Couldn’t reset the link: " + Self.message(error)
      return .failed
    }
  }

  /// Mints an open link, or with `recipient` (an account root DID) a link only
  /// that account can claim. The grant always names this space as its subject.
  private func mintPersonLink(
    runtime: any SpaceAccessRuntime, space: TonkSpace, recipient: String? = nil
  ) async throws -> (String, String?) {
    var arguments: [String: Any] = ["subject": space.subject]
    if let recipient { arguments["recipient"] = recipient }
    let result = try await runtime.accountScript(
      """
      const base = '/api/repository/' + encodeURIComponent(subject);
      const before = new Set((await api(base + '/invites')).map(row => row.targetCid));
      const minted = await api(base + '/invite',
        typeof recipient === 'string' ? {recipientRoot: recipient} : {});
      const after = await api(base + '/invites');
      return {kind: minted.kind, url: minted.url, recipientRoot: minted.recipientRoot,
              targets: after.map(row => row.targetCid).filter(cid => !before.has(cid))};
      """, arguments: arguments)
    // Ticket links live on the host serving the space, the public address's origin.
    var origins = [RuntimeLocation.home]
    if let address = spaces[space.id]?.publicAddress, let host = URL(string: address) {
      origins.append(host)
    }
    let url = try RuntimeModel.validateShareLink(
      result, subject: space.subject, origins: origins, recipient: recipient)
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
