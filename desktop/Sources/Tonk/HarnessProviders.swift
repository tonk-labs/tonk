import Foundation
import HarnessCore

@MainActor
extension HarnessModel {
  var provider: ModelProvider { saved.provider ?? .chatGPT }
  var connection: ModelConnection {
    saved.connections?[provider.rawValue] ?? ModelConnection(provider: provider)
  }
  var credentials: ModelCredentials { ModelCredentials(scope: root.lastPathComponent) }

  func configureProvider(
    _ selected: ModelProvider, connection next: ModelConnection, key: String,
    codexExecutable: String? = nil
  )
    async throws
  {
    guard storageAvailable, !connecting, !busy, !creatingSpace, !loginPending else {
      throw HarnessError.message("Finish the current operation before changing providers.")
    }
    if selected == .chatGPT, let codexExecutable {
      _ = try CodexInstallation.validate(codexExecutable)
    }
    let executableChanged = selected == .chatGPT && codexExecutable != saved.codexExecutable
    if ![.chatGPT, .claude, .disabled].contains(selected) {
      _ = try next.endpoint()
      let trimmedKey = key.trimmingCharacters(in: .whitespacesAndNewlines)
      if !trimmedKey.isEmpty { try credentials.write(trimmedKey, for: next) }
      if selected.requiresKey, try credentials.read(next).isEmpty {
        throw HarnessError.message("Enter an API key for \(selected.title).")
      }
    }
    let changed = selected != provider || next != connection
    if changed {
      let space = saved.conversation.space
      newConversation()
      guard saved.conversation.messages.isEmpty, saved.conversation.threadID == nil else {
        throw HarnessError.message(
          "Could not archive the existing conversation. Provider was not changed.")
      }
      saved.conversation.space = space
    }
    var updated = saved
    if selected == .chatGPT { updated.codexExecutable = codexExecutable }
    updated.provider = selected
    var connections = saved.connections ?? [:]
    connections[selected.rawValue] = next
    var presets = updated.modelPresets ?? []
    if !next.model.isEmpty && !presets.contains(next) { presets.append(next) }
    updated.modelPresets = presets
    updated.connections = connections
    updated.checkpointChat()
    try store.save(updated)
    saved = updated
    if changed || executableChanged || !connected {
      await connect()
    } else if selected == .claude {
      await connectClaude()
    } else if selected != .chatGPT {
      try connectAPI()
    }
  }

  func connectAPI() throws {
    if provider == .disabled {
      connected = false
      signedIn = false
      accountLabel = "Chat disabled"
      return
    }
    _ = try connection.endpoint()
    apiKey = try credentials.read(connection)
    connected = true
    signedIn = !provider.requiresKey || !apiKey.isEmpty
    accountLabel = signedIn ? "\(provider.title) · \(connection.model)" : "Add an API key"
  }

  func sendAPI(_ text: String, showsUserMessage: Bool) {
    guard canSend else { return }
    busy = true
    error = nil
    activity = "Thinking"
    saved.conversation.lastTurnStatus = nil
    if saved.conversation.threadID == nil {
      saved.conversation.threadID = "api-" + UUID().uuidString
    }
    let thread = saved.conversation.threadID!
    let turn = UUID().uuidString
    turnID = turn
    var history = APIMessage.repairingInterruptedTools(saved.conversation.apiHistory ?? [])
    history.append(APIMessage(role: "user", text: text))
    saved.conversation.apiHistory = history
    if showsUserMessage {
      saved.conversation.messages.append(ChatMessage(role: "user", text: text))
    }
    persist()
    let configuration = connection
    let tools = SpaceTools.agentDefinitions(includeCLI: RuntimeLocation.deployment != .local)
    let instructions = saved.profile.instructions + "\n\n" + spaceInstructions
    apiTask = Task { [weak self] in
      guard let self else { return }
      defer {
        self.busy = false
        self.turnID = nil
        self.activity = ""
        self.apiTask = nil
        self.persist()
      }
      do {
        try await self.apiClient.run(
          connection: configuration, key: self.apiKey,
          instructions: instructions, history: history, tools: tools,
          onHistory: {
            self.saved.conversation.apiHistory = $0
            self.saved.checkpointChat()
            try self.store.save(self.saved)
          },
          onText: { id, text in
            self.saved.conversation.appendDelta(itemID: id, text: text)
            self.activity = "Replying"
          },
          execute: { call in
            await self.callSpaceTool(
              .object([
                "threadId": .string(thread), "turnId": .string(turn), "namespace": .null,
                "tool": .string(call.name), "arguments": call.arguments,
              ]))
          })
        self.saved.conversation.lastTurnStatus = "completed"
      } catch {
        self.saved.conversation.lastTurnStatus = Task.isCancelled ? "interrupted" : "failed"
        if !Task.isCancelled { self.error = error.localizedDescription }
      }
    }
  }
}
