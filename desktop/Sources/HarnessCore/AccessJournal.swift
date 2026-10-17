import Foundation

/// What this Mac did to a space's access. The worker keeps no change feed, so
/// the overview's activity comes from here. It never holds link secrets.
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

  public init(
    id: UUID = UUID(), date: Date, subject: String, action: Action, kind: AccessLinkKind,
    target: String?
  ) {
    self.id = id
    self.date = date
    self.subject = subject
    self.action = action
    self.kind = kind
    self.target = target
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

public struct AccessJournal: Codable, Equatable, Sendable {
  public private(set) var events: [AccessEvent] = []
  /// Confirmed revocations outlive the bounded activity feed. A stale replica
  /// may reintroduce invitation facts, but cannot undo the service's receipt.
  public private(set) var revokedTargets: [String: Set<String>] = [:]
  static let limit = 500

  public init(events: [AccessEvent] = []) {
    self.events = events
    for event in events where event.action == .revoked {
      if let target = event.target { revokedTargets[event.subject, default: []].insert(target) }
    }
  }

  private enum CodingKeys: String, CodingKey { case events, revokedTargets }

  public init(from decoder: Decoder) throws {
    let values = try decoder.container(keyedBy: CodingKeys.self)
    self.init(events: try values.decode([AccessEvent].self, forKey: .events))
    for (subject, targets) in try values.decodeIfPresent(
      [String: Set<String>].self, forKey: .revokedTargets) ?? [:]
    {
      revokedTargets[subject, default: []].formUnion(targets)
    }
  }

  public mutating func confirmRevocation(subject: String, target: String) {
    revokedTargets[subject, default: []].insert(target)
  }

  public mutating func record(_ event: AccessEvent) {
    if event.action == .revoked, let target = event.target {
      confirmRevocation(subject: event.subject, target: target)
    }
    events.append(event)
    if events.count > Self.limit { events.removeFirst(events.count - Self.limit) }
  }

  public func recent(subject: String, limit: Int = 8) -> [AccessEvent] {
    Array(events.filter { $0.subject == subject }.sorted { $0.date > $1.date }.prefix(limit))
  }

  /// When this Mac made the link, directly or by resetting another, if it did.
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
