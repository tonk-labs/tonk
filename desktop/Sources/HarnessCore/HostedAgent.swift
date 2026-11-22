import Foundation

/// One row of `GET /api/agents`: an agent ChatGPT or Claude connected over
/// OAuth. Its own space's DID is the agent's identity.
public struct HostedAgent: Codable, Equatable, Sendable {
  public let did: String
  /// The agent space's routing key, which the revoke route names.
  public let key: String
  public let name: String?
  public let spaces: [HostedAgentSpace]

  public init(did: String, key: String, name: String? = nil, spaces: [HostedAgentSpace] = []) {
    self.did = did
    self.key = key
    self.name = name
    self.spaces = spaces
  }

  /// The name the approval page gave it, or a generic one.
  public var displayName: String {
    guard let name, !name.trimmingCharacters(in: .whitespaces).isEmpty else {
      return "Connected agent"
    }
    return name
  }
}

/// One space a hosted agent can reach, and how.
public struct HostedAgentSpace: Codable, Equatable, Sendable {
  public let subject: String
  public let key: String
  public let name: String?
  /// `read`, or `write` for read and write.
  public let access: String
  /// When the earliest of its grants lapses, unix seconds.
  public let expiresAt: Double?

  public init(
    subject: String, key: String, name: String? = nil, access: String = "read",
    expiresAt: Double? = nil
  ) {
    self.subject = subject
    self.key = key
    self.name = name
    self.access = access
    self.expiresAt = expiresAt
  }

  public var canWrite: Bool { access == "write" }
}
