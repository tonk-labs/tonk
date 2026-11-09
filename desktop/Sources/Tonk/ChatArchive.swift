import Foundation
import HarnessCore

/// Where a chat model reads and writes its saved state.
@MainActor
protocol ChatStateStore {
  func load() throws -> SavedState
  func save(_ state: SavedState) throws
  /// Whether another window has this chat open, so this one must not take it.
  func isOpenElsewhere(_ sessionID: String?) -> Bool
}

extension StateStore: ChatStateStore {
  func isOpenElsewhere(_ sessionID: String?) -> Bool { false }
}

/// The one `state.json` shared by the hub and every space window.
///
/// Each window's chat model writes through its own `Handle`. The archive
/// merges instead of overwriting:
/// - each chat in `sessions` keeps its newest copy, and a window never
///   updates a chat another window has open;
/// - shared settings change only where the writing window changed them since
///   it last synced, so a stale window never undoes another's settings;
/// - only windows claim chats. The open chat of the window that wrote last is
///   also kept as `conversation`, so older builds still find it.
@MainActor
final class ChatArchive {
  private let file: StateStore
  private var loaded: Result<SavedState, Error>
  private var handles: [Weak] = []

  init(directory: URL) {
    file = StateStore(directory: directory)
    loaded = Result { try StateStore(directory: directory).load() }
  }

  /// A store for one chat model. Windows claim the chats they open; the hub does not.
  func handle(claimsChats: Bool) -> Handle {
    let handle = Handle(archive: self, claimsChats: claimsChats)
    handles.removeAll { $0.handle == nil }
    handles.append(Weak(handle: handle))
    return handle
  }

  /// Saves and stops every chat model that is still open.
  func shutdown() {
    for model in handles.compactMap({ $0.handle?.model }) { model.shutdown() }
  }

  fileprivate func load(for handle: Handle) throws -> SavedState {
    var state = try loaded.get()
    // A window only takes a chat once it opens one, so it never starts out
    // holding another window's open chat.
    if handle.claimsChats, isClaimed(state.activeSessionID, except: handle) {
      state.conversation = Conversation()
      state.activeSessionID = nil
    }
    handle.base = state
    return state
  }

  fileprivate func isClaimed(_ id: String?, except handle: Handle) -> Bool {
    guard let id else { return false }
    return handles.contains {
      guard let other = $0.handle, other !== handle, other.model != nil else { return false }
      return other.claimsChats && other.claimed == id
    }
  }

  fileprivate func merge(_ incoming: SavedState, from handle: Handle) throws {
    var next = try loaded.get()
    var sessions = next.sessions ?? []
    for session in incoming.sessions ?? [] {
      if isClaimed(session.id, except: handle) { continue }
      if let index = sessions.firstIndex(where: { $0.id == session.id }) {
        if session.updatedAt > sessions[index].updatedAt { sessions[index] = session }
      } else {
        sessions.append(session)
      }
    }
    next.sessions = sessions
    let base = handle.base
    if incoming.profile != base.profile { next.profile = incoming.profile }
    if incoming.codexExecutable != base.codexExecutable {
      next.codexExecutable = incoming.codexExecutable
    }
    if incoming.modelPresets != base.modelPresets { next.modelPresets = incoming.modelPresets }
    if incoming.provider != base.provider { next.provider = incoming.provider }
    if incoming.connections != base.connections { next.connections = incoming.connections }
    if incoming.importedLegacyChats != base.importedLegacyChats {
      next.importedLegacyChats = incoming.importedLegacyChats
    }
    if handle.claimsChats {
      next.conversation = incoming.conversation
      next.activeSessionID = incoming.activeSessionID
      handle.claimed = incoming.activeSessionID
    }
    try file.save(next)
    loaded = .success(next)
    handle.base = incoming
    for other in handles.compactMap(\.handle) where other !== handle {
      other.adopt(next)
    }
  }

  @MainActor
  final class Handle: ChatStateStore {
    private weak var archive: ChatArchive?
    let claimsChats: Bool
    /// The chat model writing through this handle.
    weak var model: HarnessModel?
    /// What this handle last read or wrote, to tell its own changes from stale values.
    fileprivate var base = SavedState()
    /// The chat this window has open.
    fileprivate(set) var claimed: String?

    fileprivate init(archive: ChatArchive, claimsChats: Bool) {
      self.archive = archive
      self.claimsChats = claimsChats
    }

    func load() throws -> SavedState {
      guard let archive else { throw HarnessError.message("The chat archive is closed.") }
      return try archive.load(for: self)
    }

    func save(_ state: SavedState) throws {
      guard let archive else { throw HarnessError.message("The chat archive is closed.") }
      try archive.merge(state, from: self)
    }

    func isOpenElsewhere(_ sessionID: String?) -> Bool {
      archive?.isClaimed(sessionID, except: self) ?? false
    }

    /// Lets this window's chat go, so another window may open it.
    func release() { claimed = nil }

    /// Takes another window's saved changes: the chat list, and any setting this
    /// model has not changed itself. A model that has not opened a chat yet also
    /// refreshes its copy of the current one, so it can never write back a stale copy.
    fileprivate func adopt(_ shared: SavedState) {
      guard let model else { return }
      var next = model.saved
      next.sessions = shared.sessions
      if next.profile == base.profile {
        next.profile = shared.profile
        base.profile = shared.profile
      }
      if next.codexExecutable == base.codexExecutable {
        next.codexExecutable = shared.codexExecutable
        base.codexExecutable = shared.codexExecutable
      }
      if next.modelPresets == base.modelPresets {
        next.modelPresets = shared.modelPresets
        base.modelPresets = shared.modelPresets
      }
      if next.importedLegacyChats == base.importedLegacyChats {
        next.importedLegacyChats = shared.importedLegacyChats
        base.importedLegacyChats = shared.importedLegacyChats
      }
      if claimed == nil, let id = next.activeSessionID,
        let current = shared.sessions?.first(where: { $0.id == id })
      {
        next.conversation = current.conversation
      }
      base.sessions = shared.sessions
      if next != model.saved { model.saved = next }
    }
  }

  private struct Weak {
    weak var handle: Handle?
  }
}
