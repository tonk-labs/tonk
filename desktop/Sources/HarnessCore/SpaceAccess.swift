import Foundation

/// One row of `GET /api/repository/{subject}/invites`.
public struct InviteSummary: Decodable, Equatable, Sendable {
  public let targetCid: String
  public let kind: String
  public let recipientRoot: String?
  public let status: String
  /// The invitation entity a claim names in `xyz.tonk.membership/invitation`.
  /// Older workers omit it; usage is then unknown rather than empty.
  public let invitation: String?

  public init(
    targetCid: String, kind: String, recipientRoot: String? = nil, status: String = "active",
    invitation: String? = nil
  ) {
    self.targetCid = targetCid
    self.kind = kind
    self.recipientRoot = recipientRoot
    self.status = status
    self.invitation = invitation
  }
}

/// One row of `GET /api/account/connections`.
public struct AgentConnection: Decodable, Equatable, Sendable {
  public struct Installation: Decodable, Equatable, Sendable {
    public let id: String
    public let name: String
    public init(id: String, name: String) {
      self.id = id
      self.name = name
    }
  }
  public let id: String
  public let subject: String
  public let label: String
  /// Authorization origin, not an account kind or transport.
  public let kind: String?
  public let recipient: String?
  public let expiresAt: Double
  public let status: String
  public let confirmed: Bool
  public let spaceName: String?
  public let installations: [Installation]

  public init(
    id: String, subject: String, label: String = "", expiresAt: Double, status: String = "active",
    confirmed: Bool = false, spaceName: String? = nil, installations: [Installation] = [],
    kind: String? = nil, recipient: String? = nil
  ) {
    self.id = id
    self.subject = subject
    self.label = label
    self.kind = kind
    self.recipient = recipient
    self.expiresAt = expiresAt
    self.status = status
    self.confirmed = confirmed
    self.spaceName = spaceName
    self.installations = installations
  }

  enum CodingKeys: String, CodingKey {
    case id, subject, label, expiresAt, status, confirmed, spaceName, installations, kind, recipient
  }

  public init(from decoder: Decoder) throws {
    let container = try decoder.container(keyedBy: CodingKeys.self)
    id = try container.decode(String.self, forKey: .id)
    subject = try container.decode(String.self, forKey: .subject)
    label = try container.decodeIfPresent(String.self, forKey: .label) ?? ""
    kind = try container.decodeIfPresent(String.self, forKey: .kind)
    recipient = try container.decodeIfPresent(String.self, forKey: .recipient)
    expiresAt = try container.decode(Double.self, forKey: .expiresAt)
    status = try container.decode(String.self, forKey: .status)
    confirmed = try container.decodeIfPresent(Bool.self, forKey: .confirmed) ?? false
    spaceName = try container.decodeIfPresent(String.self, forKey: .spaceName)
    installations = try container.decodeIfPresent([Installation].self, forKey: .installations) ?? []
  }

  /// Only the worker's explicit terminal origin identifies CLI authorization.
  /// A client name or confirmation does not establish a provisioned account.
  public var isTerminalAuthorization: Bool { kind == "terminal" }

  public var isLive: Bool { status == "active" || status == "partial" }
  /// Something actually connected through the grant: a CLI authorization, an agent's
  /// confirmation, or a registered client. A minted link nobody has used is only a link.
  public var hasConnected: Bool { isTerminalAuthorization || confirmed || !installations.isEmpty }
}

/// A roster row from the `tonk:member` facts on a space's content branch.
public struct SpaceMember: Equatable, Sendable, Identifiable {
  public let did: String
  public let name: String?
  public let role: String?
  public let invitation: String?
  public var id: String { did }

  public init(did: String, name: String?, role: String?, invitation: String?) {
    self.did = did
    self.name = name
    self.role = role
    self.invitation = invitation
  }

  public var isFounder: Bool { role == "tonk:founder" }
  public var displayName: String {
    let trimmed = name?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    return trimmed.isEmpty ? "Member " + did.suffix(6) : trimmed
  }
  public var initial: String {
    let trimmed = name?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    return trimmed.first.map { String($0).uppercased() } ?? "?"
  }
}

/// A space view card: the home view or a model's directory.
public struct SpaceViewEntry: Equatable, Sendable, Identifiable {
  public let model: String
  public let title: String
  public let isHome: Bool
  public var id: String { model }

  public init(model: String, title: String, isHome: Bool) {
    self.model = model
    self.title = title
    self.isHome = isHome
  }

  /// `/space/{did}` for the home, `/space/{did}/{model}` for a directory.
  public func url(in space: TonkSpace) -> URL {
    isHome ? space.url : space.url.appendingPathComponent(model)
  }

  /// A readable name for a model URI such as `tonk:board` or `id:cycle-14`.
  public static func title(forModel model: String) -> String {
    let tail = model.split(whereSeparator: { ":/".contains($0) }).last.map(String.init) ?? model
    let words = tail.replacingOccurrences(of: "-", with: " ")
      .replacingOccurrences(of: "_", with: " ")
    guard let first = words.first else { return model }
    return first.uppercased() + words.dropFirst()
  }
}

public enum AccessLinkKind: String, Codable, Sendable {
  case person
  case scoped
  case agent
  case publicRead
}

/// One manageable grant on a space, presented the same way whatever minted it.
public struct AccessLink: Equatable, Sendable, Identifiable {
  public let kind: AccessLinkKind
  public let subject: String
  /// Invite target CID, agent connection id, or the space DID for a publication.
  public let target: String
  public let audience: String
  public let expiresAt: Date?
  public let status: String
  /// `nil` when the worker cannot say who used it.
  public let usedBy: [String]?
  /// The invitation a member names when they claim this link; `nil` for agents,
  /// the public link and older workers.
  public let invitation: String?

  public init(
    kind: AccessLinkKind, subject: String, target: String, audience: String = "Anyone",
    expiresAt: Date? = nil, status: String = "active", usedBy: [String]?,
    invitation: String? = nil
  ) {
    self.kind = kind
    self.subject = subject
    self.target = target
    self.audience = audience
    self.expiresAt = expiresAt
    self.status = status
    self.usedBy = usedBy
    self.invitation = invitation
  }

  public var id: String { kind.rawValue + ":" + target }
  public var forAgents: Bool { kind == .agent }

  public var title: String {
    switch kind {
    case .person: "Default link"
    case .scoped: "Link for \(audience)"
    case .agent: "Agent link"
    case .publicRead: "Public link"
    }
  }

  public var ability: String { kind == .publicRead ? "Read-only" : "Read and write" }

  /// Open invites can be replaced in place; other kinds are revoked.
  public var canReset: Bool { kind == .person }

  /// Every link opens the whole space; only the public one says so, to set it
  /// apart from the read-and-write links beside it.
  public func detail(now: Date) -> String {
    [
      audience == "Anyone" ? "Anyone with the link" : audience, ability,
      kind == .publicRead ? "Whole space" : nil, Self.expiry(expiresAt, status: status, now: now),
    ].compactMap { $0 }.joined(separator: " · ")
  }

  public func usage(madeAt: Date?, now: Date) -> String? {
    if kind == .publicRead { return "Anyone with the address can read" }
    let made = madeAt.map { "Made " + AccessText.relative($0, now: now).lowercasedFirst() }
    guard let usedBy else { return made }
    if kind == .agent {
      // Per-call MCP authority does not register a client installation.
      // Installation metadata therefore cannot establish whether a link was used.
      if usedBy.isEmpty {
        return [made, "No registered clients"].compactMap { $0 }.joined(separator: " · ")
      }
      return "Connected clients: " + AccessText.list(usedBy)
    }
    if usedBy.isEmpty { return [made, "Not used yet"].compactMap { $0 }.joined(separator: " · ") }
    return "Used by " + AccessText.list(usedBy)
  }

  public static func expiry(_ date: Date?, status: String, now: Date) -> String {
    if status == "revoked" { return "Revoked" }
    guard let date else { return "Never expires" }
    if status == "expired" || date <= now { return "Expired" }
    let days = Int((date.timeIntervalSince(now) / 86_400).rounded(.up))
    if days <= 1 {
      let hours = max(1, Int((date.timeIntervalSince(now) / 3600).rounded(.up)))
      return hours >= 24 ? "Expires in 1 day" : "Expires in \(hours) hour\(hours == 1 ? "" : "s")"
    }
    return "Expires in \(days) days"
  }
}

/// Everything the overview shows for one space, assembled from worker reads.
public struct SpaceAccess: Equatable, Sendable {
  public var members: [SpaceMember]
  public var links: [AccessLink]
  public var agents: [AgentConnection]
  public var views: [SpaceViewEntry]
  public var isPublic: Bool
  /// The signed-in account's DID, to say "Your space".
  public var selfDid: String?
  /// `{space host}/space/{did}`, which anyone can read once the space is public.
  public var publicAddress: String?

  public init(
    members: [SpaceMember] = [], links: [AccessLink] = [], agents: [AgentConnection] = [],
    views: [SpaceViewEntry] = [], isPublic: Bool = false, selfDid: String? = nil
  ) {
    self.members = members
    self.links = links
    self.agents = agents
    self.views = views
    self.isPublic = isPublic
    self.selfDid = selfDid
  }

  public var liveAgents: [AgentConnection] { agents.filter(\.isLive) }

  /// "Kai's space · 6 people, 3 agents".
  public var peopleSummary: String {
    let founder = members.first(where: \.isFounder)
    let owner: String? =
      founder.map { $0.did == selfDid ? "Your space" : $0.displayName + "’s space" }
    let people = members.count == 1 ? "1 person" : "\(members.count) people"
    let agentCount = liveAgents.count
    let counts =
      agentCount == 0
      ? people : people + ", \(agentCount) access grant\(agentCount == 1 ? "" : "s")"
    return [owner, counts].compactMap { $0 }.joined(separator: " · ")
  }

  /// Builds link rows from the invite and agent listings plus the roster.
  public static func links(
    subject: String, invites: [InviteSummary], agents: [AgentConnection],
    members: [SpaceMember], isPublic: Bool, names: [String: String] = [:]
  ) -> [AccessLink] {
    var rows: [AccessLink] = []
    if isPublic {
      rows.append(AccessLink(kind: .publicRead, subject: subject, target: subject, usedBy: nil))
    }
    for invite in invites where invite.status == "active" {
      let usedBy = invite.invitation.map { invitation in
        members.filter { $0.invitation == invitation }.map(\.displayName)
      }
      switch invite.kind {
      case "open":
        rows.append(
          AccessLink(
            kind: .person, subject: subject, target: invite.targetCid, usedBy: usedBy,
            invitation: invite.invitation))
      case "scoped":
        // The roster names someone who joined; a saved contact names someone invited.
        let recipient = invite.recipientRoot.flatMap { root in
          members.first(where: { $0.did == root })?.displayName ?? names[root]
        }
        let fallback = invite.recipientRoot.map { "account " + $0.suffix(6) } ?? "one account"
        rows.append(
          AccessLink(
            kind: .scoped, subject: subject, target: invite.targetCid,
            audience: recipient ?? fallback, usedBy: usedBy, invitation: invite.invitation))
      default:
        // Legacy rows carry no execution kind; still revocable as a link.
        rows.append(
          AccessLink(
            kind: .person, subject: subject, target: invite.targetCid, usedBy: usedBy,
            invitation: invite.invitation))
      }
    }
    for agent in agents where agent.subject == subject && agent.isLive {
      rows.append(
        AccessLink(
          kind: .agent, subject: subject, target: agent.id,
          expiresAt: Date(timeIntervalSince1970: agent.expiresAt), status: agent.status,
          usedBy: agent.installations.map(\.name)))
    }
    return rows
  }
}

public enum AccessText {
  /// "Just now", "4 min ago", "1 hour ago", "Yesterday", "2 days ago".
  public static func relative(_ date: Date, now: Date, calendar: Calendar = .current) -> String {
    let seconds = now.timeIntervalSince(date)
    if seconds < 60 { return "Just now" }
    if seconds < 3600 { return "\(Int(seconds / 60)) min ago" }
    if seconds < 86_400 && calendar.isDate(date, inSameDayAs: now) {
      let hours = Int(seconds / 3600)
      return hours == 1 ? "1 hour ago" : "\(hours) hours ago"
    }
    let days =
      calendar.dateComponents(
        [.day], from: calendar.startOfDay(for: date), to: calendar.startOfDay(for: now)
      ).day ?? 0
    if days <= 1 { return "Yesterday" }
    if days < 7 { return "\(days) days ago" }
    return date.formatted(.dateTime.month(.abbreviated).day())
  }

  /// "Mira", "Mira and Lu", "Mira, Lu and Kai", "Mira, Lu and 3 others".
  public static func list(_ names: [String]) -> String {
    switch names.count {
    case 0: return ""
    case 1: return names[0]
    case 2: return names[0] + " and " + names[1]
    case 3: return names[0] + ", " + names[1] + " and " + names[2]
    default: return names[0] + ", " + names[1] + " and \(names.count - 2) others"
    }
  }

  /// Two-letter initials for an account name.
  public static func initials(_ name: String) -> String {
    let words = name.split(whereSeparator: \.isWhitespace)
    let letters = words.prefix(2).compactMap(\.first).map { String($0).uppercased() }
    return letters.isEmpty ? "?" : letters.joined()
  }
}

extension String {
  func lowercasedFirst() -> String {
    guard let first else { return self }
    return first.lowercased() + dropFirst()
  }
}

/// How one member's access to a space can be taken away. Access comes from the
/// link they claimed, so removing someone means revoking that link.
public enum MemberRemoval: Equatable, Sendable {
  /// A link made for this person alone: revoking it removes only them.
  case personal(AccessLink)
  /// A link anyone could use. Resetting it removes everyone who joined with it.
  case shared(AccessLink, alsoRemoves: [String])
  /// The founder holds the space itself and cannot be removed.
  case founder
  /// The link they joined with is gone or the worker does not say which it was.
  case unknown
}

extension SpaceAccess {
  public func member(_ did: String) -> SpaceMember? { members.first { $0.did == did } }

  /// `nil` when `did` is not a member of this space.
  public func removal(of did: String) -> MemberRemoval? {
    guard let member = member(did) else { return nil }
    if member.isFounder { return .founder }
    guard let invitation = member.invitation,
      let link = links.first(where: { $0.invitation == invitation })
    else { return .unknown }
    if link.kind == .scoped { return .personal(link) }
    let others = members.filter { $0.invitation == invitation && $0.did != did }
    return .shared(link, alsoRemoves: others.map(\.displayName))
  }
}
