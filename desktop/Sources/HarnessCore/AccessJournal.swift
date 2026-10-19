import Foundation

/// What the account did to a space's access, from any of its desktops. The
/// worker keeps no change feed, so each desktop records these on the account
/// branch as `tonk:access-event` rows. It never holds link secrets.
public struct AccessEvent: Codable, Equatable, Sendable, Identifiable {
  public enum Action: String, Codable, Sendable {
    case made, reset, revoked, published, unpublished
  }
  public let id: UUID
  public let date: Date
  public let subject: String
  public let action: Action
  public let kind: AccessLinkKind
  /// Invite target CID, agent connection id, or the space DID.
  public let target: String?
  /// On a reset, the target of the plain link it revoked.
  public let replaced: String?

  public init(
    id: UUID = UUID(), date: Date, subject: String, action: Action, kind: AccessLinkKind,
    target: String?, replaced: String? = nil
  ) {
    self.id = id
    self.date = date
    self.subject = subject
    self.action = action
    self.kind = kind
    self.target = target
    self.replaced = replaced
  }

  /// The event's entity on the account branch.
  public var entity: String { "urn:uuid:" + id.uuidString.lowercased() }

  /// The targets this event proves no longer work.
  var revokes: [String] {
    switch action {
    case .revoked: target.map { [$0] } ?? []
    case .reset: replaced.map { [$0] } ?? []
    default: []
    }
  }

  public var sentence: String {
    let link =
      switch kind {
      case .person: "a plain link"
      case .scoped: "a link for one account"
      case .agent: "a link for agents"
      case .publicRead: "a public read link"
      }
    return switch action {
    case .made: "You made \(link)"
    case .reset: "You reset the plain link"
    case .revoked: "You revoked \(link)"
    case .published: "You made the space public"
    case .unpublished: "You made the space private"
    }
  }
}

/// This Mac's copy of the account's access activity: the events read back
/// from the account branch plus any it has not managed to write there yet.
public struct AccessJournal: Codable, Equatable, Sendable {
  public private(set) var events: [AccessEvent] = []
  /// Confirmed revocations outlive the bounded activity feed. A stale replica
  /// may reintroduce invitation facts, but cannot undo the service's receipt.
  public private(set) var revokedTargets: [String: Set<String>] = [:]
  /// Events recorded here and not yet on the account branch.
  public private(set) var unsynced: Set<UUID> = []
  static let limit = 500

  public init(events: [AccessEvent] = []) {
    self.events = events
    for event in events {
      for target in event.revokes { revokedTargets[event.subject, default: []].insert(target) }
    }
  }

  private enum CodingKeys: String, CodingKey { case events, revokedTargets, unsynced }

  public init(from decoder: Decoder) throws {
    let values = try decoder.container(keyedBy: CodingKeys.self)
    self.init(events: try values.decode([AccessEvent].self, forKey: .events))
    for (subject, targets) in try values.decodeIfPresent(
      [String: Set<String>].self, forKey: .revokedTargets) ?? [:]
    {
      revokedTargets[subject, default: []].formUnion(targets)
    }
    // A journal written before activity synced kept everything on this Mac,
    // so all of it still has to reach the account.
    unsynced = try values.decodeIfPresent(Set<UUID>.self, forKey: .unsynced)
      ?? Set(events.map(\.id))
  }

  public mutating func confirmRevocation(subject: String, target: String) {
    revokedTargets[subject, default: []].insert(target)
  }

  /// Adds an event made on this Mac; it stays unsynced until `markSynced`.
  public mutating func record(_ event: AccessEvent) {
    insert(event)
    unsynced.insert(event.id)
    trim()
  }

  /// Folds in events read from the account branch. Ones already known are
  /// kept as they are, so a replayed read changes nothing.
  public mutating func merge(_ remote: [AccessEvent]) {
    let known = Set(events.map(\.id))
    for event in remote where !known.contains(event.id) { insert(event) }
    events.sort { $0.date < $1.date }
    trim()
  }

  public mutating func markSynced(_ ids: some Sequence<UUID>) {
    unsynced.subtract(ids)
  }

  /// Unsynced events, oldest first.
  public var pending: [AccessEvent] { events.filter { unsynced.contains($0.id) } }

  private mutating func insert(_ event: AccessEvent) {
    for target in event.revokes { confirmRevocation(subject: event.subject, target: target) }
    events.append(event)
  }

  /// Bounds the feed, dropping the oldest synced events before any that
  /// still have to reach the account.
  private mutating func trim() {
    var excess = events.count - Self.limit
    guard excess > 0 else { return }
    events.removeAll { event in
      guard excess > 0, !unsynced.contains(event.id) else { return false }
      excess -= 1
      return true
    }
    if excess > 0 {
      unsynced.subtract(events.prefix(excess).map(\.id))
      events.removeFirst(excess)
    }
  }

  public func recent(subject: String, limit: Int = 8) -> [AccessEvent] {
    Array(events.filter { $0.subject == subject }.sorted { $0.date > $1.date }.prefix(limit))
  }

  /// When the account made the link, directly or by resetting another.
  public func madeAt(target: String) -> Date? {
    events.first(where: { $0.target == target && [.made, .reset].contains($0.action) })?.date
  }

  public static func load(from url: URL) -> AccessJournal {
    guard let data = try? Data(contentsOf: url) else { return AccessJournal() }
    let decoder = JSONDecoder()
    decoder.dateDecodingStrategy = .iso8601
    return (try? decoder.decode(Self.self, from: data)) ?? AccessJournal()
  }

  public func save(to url: URL) throws {
    let encoder = JSONEncoder()
    encoder.dateEncodingStrategy = .iso8601
    try encoder.encode(self).write(to: url, options: .atomic)
  }
}
