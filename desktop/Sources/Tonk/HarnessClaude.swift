import Foundation
import HarnessCore

@MainActor
extension HarnessModel {
  var claudeWorkspace: URL { root.appendingPathComponent("ClaudeWorkspace", isDirectory: true) }

  func connectClaude() async {
    claudeModels = []
    claudeModelsError = nil
    do {
      try await claudeClient.discover()
      signedIn = try await claudeClient.authenticated(workspace: claudeWorkspace)
      connected = true
      accountLabel = signedIn ? "Claude connected" : "Sign in with Claude"
      if signedIn { await refreshClaudeModels() }
    } catch { self.error = error.localizedDescription }
  }

  func refreshClaudeModels() async {
    guard provider == .claude, signedIn, !claudeModelsLoading else { return }
    claudeModelsLoading = true
    defer { claudeModelsLoading = false }
    do {
      claudeModels = try await claudeClient.models(workspace: claudeWorkspace)
      claudeModelsError = nil
    } catch {
      claudeModels = []
      claudeModelsError = error.localizedDescription
    }
  }

  func signInClaude() async {
    guard connected, !loginPending, !busy else { return }
    loginPending = true
    error = nil
    apiTask = Task { [self] in
      defer {
        loginPending = false
        apiTask = nil
      }
      do {
        try await claudeClient.login(workspace: claudeWorkspace)
        signedIn = try await claudeClient.authenticated(workspace: claudeWorkspace)
        accountLabel = signedIn ? "Claude connected" : "Not signed in"
        if !signedIn {
          throw HarnessError.message("Sign in with a Claude subscription, then reconnect.")
        }
        await refreshClaudeModels()
      } catch {
        if !Task.isCancelled { self.error = error.localizedDescription }
      }
    }
  }

  func sendClaude(_ text: String, showsUserMessage: Bool) {
    guard canSend else { return }
    busy = true
    error = nil
    activity = "Thinking"
    let resume = saved.conversation.claudeSessionStarted == true
    let session = saved.conversation.threadID ?? UUID().uuidString
    let turn = UUID().uuidString
    saved.conversation.threadID = session
    saved.conversation.lastTurnStatus = nil
    turnID = turn
    if showsUserMessage {
      saved.conversation.messages.append(ChatMessage(role: "user", text: text))
    }
    persist()
    apiTask = Task { [self] in
      let tools = SpaceTools.agentDefinitions(includeCLI: RuntimeLocation.deployment != .local).map
      { tool in
        JSONValue.object([
          "name": tool["name"], "description": tool["description"],
          "inputSchema": tool["inputSchema"],
        ])
      }
      let bridge = LocalRuntimeBridge(allowsWrites: true, agentTools: tools) { [weak self] name, arguments in
        guard let self else { return SpaceTools.response("Conversation closed.", success: false) }
        return await self.callSpaceTool(
          .object([
            "threadId": .string(session), "turnId": .string(turn), "namespace": .null,
            "tool": .string(name), "arguments": arguments,
          ]))
      }
      defer {
        bridge.stop()
        busy = false
        turnID = nil
        activity = ""
        apiTask = nil
        persist()
      }
      do {
        let connection = try await bridge.start()
        let config = JSONValue.object([
          "mcpServers": .object([
            "tonk": .object([
              "type": .string("http"), "url": .string((connection["url"].string ?? "") + "/mcp"),
              "headers": .object([
                "Authorization": .string("Bearer " + (connection["token"].string ?? ""))
              ]),
            ])
          ])
        ])
        try await claudeClient.turn(
          prompt: text, session: session, resume: resume,
          model: self.connection.model, workspace: claudeWorkspace,
          mcp: String(decoding: try JSONEncoder().encode(config), as: UTF8.self),
          instructions: saved.profile.instructions + "\n\n" + spaceInstructions,
          onSession: {
            self.saved.conversation.claudeSessionStarted = true
            self.persist()
          }
        ) { id, delta in
          self.saved.conversation.appendDelta(itemID: turn + id, text: delta)
          self.activity = "Replying"
        }
        saved.conversation.lastTurnStatus = "completed"
      } catch {
        saved.conversation.lastTurnStatus = Task.isCancelled ? "interrupted" : "failed"
        if error is ClaudeCodeError {
          signedIn = false
          accountLabel = "Sign in with Claude"
        }
        if saved.conversation.claudeSessionStarted != true { saved.conversation.threadID = nil }
        if !Task.isCancelled { self.error = error.localizedDescription }
      }
    }
  }
}
