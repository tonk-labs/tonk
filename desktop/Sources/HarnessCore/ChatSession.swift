import Foundation

/// A chat owns its transcript, pinned space, and model configuration. Never contains keys.
public struct ChatSession: Codable, Equatable, Identifiable, Sendable {
  public var id: String
  public var conversation: Conversation
  public var provider: ModelProvider
  public var connection: ModelConnection?
  public var updatedAt: Date
  public var title: String {
    let text = conversation.messages.first(where: { $0.role == "user" })?.text ?? "New chat"
    return String(text.replacingOccurrences(of: "\n", with: " ").prefix(80))
  }
  public init(id: String = UUID().uuidString, state: SavedState, updatedAt: Date = Date()) {
    self.id = id
    conversation = state.conversation
    provider = state.provider ?? .chatGPT
    connection = state.connections?[provider.rawValue]
    self.updatedAt = updatedAt
  }
}

extension SavedState {
  public mutating func checkpointChat() {
    let id = activeSessionID ?? UUID().uuidString
    activeSessionID = id
    let item = ChatSession(id: id, state: self)
    var list = sessions ?? []
    if let index = list.firstIndex(where: { $0.id == id }) {
      // Reading or reopening a chat should not move it to the top.
      if list[index].conversation == item.conversation && list[index].provider == item.provider
        && list[index].connection == item.connection
      {
        return
      }
      list[index] = item
    } else {
      list.append(item)
    }
    sessions = list
  }

  public func chats(in spaceID: String?) -> [ChatSession] {
    (sessions ?? []).filter { $0.conversation.space?.id == spaceID }
      .sorted { $0.updatedAt > $1.updatedAt }
  }

  public mutating func restoreChat(_ id: String) throws {
    checkpointChat()
    guard let session = sessions?.first(where: { $0.id == id }) else {
      throw HarnessError.message("This chat could not be found.")
    }
    activeSessionID = session.id
    conversation = session.conversation
    provider = session.provider
    if let connection = session.connection {
      var configurations = connections ?? [:]
      configurations[session.provider.rawValue] = connection
      connections = configurations
    }
  }
}
