import Foundation
import HarnessCore

@MainActor
extension HarnessModel {
  func importChatHistory() throws {
    guard saved.importedLegacyChats != true else { return }
    var next = saved
    next.checkpointChat()
    let archiveRoot = root.appendingPathComponent("Conversations")
    if FileManager.default.fileExists(atPath: archiveRoot.path) {
      for directory in try FileManager.default.contentsOfDirectory(
        at: archiveRoot, includingPropertiesForKeys: [.contentModificationDateKey])
      {
        guard UUID(uuidString: directory.lastPathComponent) != nil else { continue }
        let old = try StateStore(directory: directory).load()
        guard !old.conversation.messages.isEmpty else { continue }
        let date =
          try directory.resourceValues(forKeys: [.contentModificationDateKey])
          .contentModificationDate ?? .distantPast
        let session = ChatSession(id: directory.lastPathComponent, state: old, updatedAt: date)
        if !(next.sessions ?? []).contains(where: { $0.id == session.id }) {
          next.sessions?.append(session)
        }
      }
    }
    next.importedLegacyChats = true
    try store.save(next)
    saved = next
  }

  func openChat(_ id: String, connectInBackground: Bool = false) async -> Bool {
    guard storageAvailable, !busy, !creatingSpace, !connecting, !loginPending else { return false }
    do {
      var next = saved
      try next.restoreChat(id)
      try store.save(next)
      saved = next
      toolActivity = []
      spaceCreationError = nil
      if connectInBackground { self.connectInBackground() } else { await connect() }
      return true
    } catch {
      self.error = error.localizedDescription
      return false
    }
  }

  func enterSpace(_ space: TonkSpace) async -> Bool {
    guard await openSpaceChat(space, connectInBackground: true) else { return false }
    openedSpace = space
    showingChat = false
    if !connected { connectInBackground() }
    return true
  }

  func startSpaceChat() {
    guard let openedSpace, saved.conversation.space?.id == openedSpace.id,
      !busy, !creatingSpace, !connecting, !loginPending, storageAvailable
    else { return }
    let previous = saved.activeSessionID
    newConversation()
    if saved.activeSessionID != previous { showingChat = true }
  }

  func openSpaceChat(_ space: TonkSpace, connectInBackground: Bool = false) async -> Bool {
    guard storageAvailable, !busy, !creatingSpace, !connecting, !loginPending else { return false }
    if saved.conversation.space?.id == space.id { return true }
    if let existing = saved.chats(in: space.id).first {
      return await openChat(existing.id, connectInBackground: connectInBackground)
    }
    attachSpace(space)
    return saved.conversation.space?.id == space.id
  }
}
