import AppKit
import HarnessCore
import SwiftUI

@MainActor
final class HarnessModel: ObservableObject {
  @Published var saved = SavedState()
  @Published var showingChat = false
  @Published var openedSpace: TonkSpace?
  @Published var subscriptionModels: [SubscriptionModel] = []
  @Published var subscriptionModelsLoading = false
  @Published var modelCatalogError: String?
  @Published var connected = false
  @Published var connecting = false
  @Published private(set) var codexDiscoveryFailed = false
  @Published var signedIn = false
  @Published var accountLabel = "Not signed in"
  @Published var busy = false
  @Published var activity = ""
  @Published var error: String?
  @Published var loginPending = false
  weak var runtime: RuntimeModel?
  var cliAdapters: [String: TonkCLI] = [:]
  @Published var creatingSpace = false
  @Published var spaceCreationError: String?
  @Published var toolActivity: [String] = []
  let client = AppServerClient()
  let claudeClient: ClaudeCodeClient
  @Published var claudeModels: [ClaudeModel] = []
  @Published var claudeModelsLoading = false
  @Published var claudeModelsError: String?
  let apiClient: APIModelClient
  var apiTask: Task<Void, Never>?
  var apiKey = ""
  let root: URL
  let store: StateStore
  private var loginID: String?
  @Published var turnID: String?
  var activeTurnID: String? { turnID }
  var resumed = false
  private var saveTask: Task<Void, Never>?
  var storageAvailable = true

  init(
    directory: URL? = nil, apiClient: APIModelClient? = nil, claudeClient: ClaudeCodeClient? = nil
  ) {
    self.claudeClient = claudeClient ?? ClaudeCodeClient()
    self.apiClient = apiClient ?? APIModelClient()
    let arguments = ProcessInfo.processInfo.arguments
    if let directory {
      root = directory
    } else if let index = arguments.firstIndex(of: "--data-dir"),
      arguments.indices.contains(index + 1)
    {
      root = URL(fileURLWithPath: arguments[index + 1], isDirectory: true)
    } else {
      root = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        .appendingPathComponent(RuntimeLocation.deployment.dataDirectory, isDirectory: true)
    }
    store = StateStore(directory: root)
    do { saved = try store.load() } catch {
      storageAvailable = false
      self.error =
        "Could not read your saved conversation. It has been preserved at \(root.path)/state.json. \(error.localizedDescription)"
    }
    if storageAvailable {
      do { try importChatHistory() } catch {
        storageAvailable = false
        self.error = "Could not load chat history: \(error.localizedDescription)"
      }
    }
    client.onToolCall = { [weak self] params in
      guard let self else {
        return SpaceTools.response("The conversation is closed.", success: false)
      }
      return await self.callSpaceTool(params)
    }
    client.onNotification = { [weak self] method, params in self?.receive(method, params) }
    client.onDisconnect = { [weak self] message in
      guard let self else { return }
      guard self.provider == .chatGPT else { return }
      self.connected = false
      self.busy = false
      self.resumed = false
      self.turnID = nil
      self.loginID = nil
      self.loginPending = false
      self.error = message
      self.persist()
    }
  }

  var canSend: Bool {
    connected && signedIn && !connecting && !busy && !creatingSpace && !loginPending && storageAvailable
  }

  func connect() async {
    guard !connecting else { return }
    connecting = true
    await establishProviderConnection()
  }

  func connectInBackground() {
    guard !connecting else { return }
    connecting = true
    Task { await establishProviderConnection() }
  }

  private func establishProviderConnection() async {
    codexDiscoveryFailed = false
    defer { connecting = false }
    if storageAvailable { error = nil }
    claudeClient.stop()
    client.stop()
    connected = false
    signedIn = false
    subscriptionModels = []
    modelCatalogError = nil
    apiKey = ""
    accountLabel = "Not connected"
    resumed = false
    busy = false
    loginID = nil
    loginPending = false
    turnID = nil
    if provider == .claude {
      await connectClaude()
      return
    }
    if provider != .chatGPT {
      do { try connectAPI() } catch { self.error = error.localizedDescription }
      return
    }
    do {
      let installation: CodexInstallation
      do {
        installation = try await CodexInstallation.discover(selectedPath: saved.codexExecutable)
      } catch {
        codexDiscoveryFailed = !(error is CancellationError)
        throw error
      }
      try await client.start(
        executable: installation.executable,
        home: root.appendingPathComponent("Codex"),
        workspace: root.appendingPathComponent("Workspace"), searchPath: installation.searchPath)
      connected = true
      try await refreshAccount()
    } catch { self.error = error.localizedDescription }
  }

  func refreshAccount() async throws {
    guard provider == .chatGPT else { return }
    let result = try await client.request(
      "account/read", params: .object(["refreshToken": .bool(false)]))
    signedIn = result["account"]["type"].string == "chatgpt"
    accountLabel = signedIn ? "ChatGPT connected" : "Not signed in"
    if signedIn { await refreshSubscriptionModels() }
  }

  func signIn() async {
    if provider == .claude {
      await signInClaude()
      return
    }
    guard connected, !loginPending else { return }
    loginPending = true
    error = nil
    do {
      let result = try await client.request(
        "account/login/start", params: .object(["type": .string("chatgpt")]))
      loginID = result["loginId"].string
      guard let address = result["authUrl"].string, let url = URL(string: address),
        url.scheme == "https"
      else {
        throw HarnessError.message("Codex did not return a valid sign-in address.")
      }
      guard NSWorkspace.shared.open(url) else {
        throw HarnessError.message("Could not open your browser for sign-in.")
      }
    } catch {
      if let loginID {
        _ = try? await client.request(
          "account/login/cancel", params: .object(["loginId": .string(loginID)]))
      }
      loginPending = false
      loginID = nil
      self.error = error.localizedDescription
    }
  }

  func cancelLogin() async {
    if provider == .claude {
      apiTask?.cancel()
      claudeClient.stop()
      return
    }
    guard let loginID else { return }
    self.loginID = nil
    loginPending = false
    error = nil
    do {
      _ = try await client.request(
        "account/login/cancel", params: .object(["loginId": .string(loginID)]))
    } catch { self.error = error.localizedDescription }
  }

  func signOut() async {
    guard [.chatGPT, .claude].contains(provider), connected,
      !busy, !creatingSpace, !connecting, !loginPending,
      !subscriptionModelsLoading, !claudeModelsLoading
    else { return }
    error = nil
    do {
      if provider == .claude {
        try await claudeClient.logout(workspace: claudeWorkspace)
      } else {
        _ = try await client.request("account/logout")
      }
      signedIn = false
      accountLabel = "Not signed in"
      subscriptionModels = []
      modelCatalogError = nil
      claudeModels = []
      claudeModelsError = nil
      resumed = false
    } catch { self.error = error.localizedDescription }
  }

  func send(_ text: String, showsUserMessage: Bool = true) async {
    let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
    guard canSend, !trimmed.isEmpty else { return }
    if provider == .claude {
      sendClaude(trimmed, showsUserMessage: showsUserMessage)
      return
    }
    if provider != .chatGPT {
      sendAPI(trimmed, showsUserMessage: showsUserMessage)
      return
    }
    busy = true
    saved.conversation.lastTurnStatus = nil
    error = nil
    activity = "Thinking"
    do {
      var parameters: [String: JSONValue] = [
        "cwd": .string(root.appendingPathComponent("Workspace").path),
        "developerInstructions": .string(saved.profile.instructions + spaceInstructions),
        "sandbox": .string("read-only"), "approvalPolicy": .string("never"),
      ]
      if !connection.model.isEmpty { parameters["model"] = .string(connection.model) }
      if let threadID = saved.conversation.threadID {
        if !resumed {
          parameters["threadId"] = .string(threadID)
          let response = try await client.request("thread/resume", params: .object(parameters))
          saved.conversation.resolvedModel = response["model"].string
          resumed = true
        }
      } else {
        parameters["dynamicTools"] = .array(
          SpaceTools.agentDefinitions(includeCLI: RuntimeLocation.deployment != .local))
        let result = try await client.request("thread/start", params: .object(parameters))
        saved.conversation.resolvedModel = result["model"].string
        guard let threadID = result["thread"]["id"].string else {
          throw HarnessError.message("The agent did not create a conversation.")
        }
        saved.conversation.threadID = threadID
        resumed = true
      }
      guard let threadID = saved.conversation.threadID else {
        throw HarnessError.message("No active conversation.")
      }
      if showsUserMessage {
        saved.conversation.messages.append(ChatMessage(role: "user", text: trimmed))
      }
      persist()
      let result = try await client.request(
        "turn/start",
        params: .object([
          "threadId": .string(threadID),
          "input": .array([
            .object([
              "type": .string("text"), "text": .string(trimmed), "text_elements": .array([]),
            ])
          ]),
        ]))
      // Completion can arrive before the request continuation is scheduled.
      if busy { turnID = result["turn"]["id"].string }
    } catch {
      busy = false
      activity = ""
      turnID = nil
      self.error = error.localizedDescription
      persist()
    }
  }

  func stopTurn() async {
    if let apiTask {
      activity = "Stopping"
      apiTask.cancel()
      return
    }
    client.cancelTools()
    guard let threadID = saved.conversation.threadID, let turnID else { return }
    activity = "Stopping"
    do {
      _ = try await client.request(
        "turn/interrupt",
        params: .object([
          "threadId": .string(threadID), "turnId": .string(turnID),
        ]))
    } catch {
      self.error = error.localizedDescription
      activity = "Stop failed"
    }
  }

  var canStop: Bool { busy && turnID != nil }

  func newConversation() {
    guard storageAvailable, !busy, !creatingSpace, !loginPending, !connecting else { return }
    do {
      var next = saved
      next.checkpointChat()
      let space = next.conversation.space
      next.conversation = Conversation()
      next.conversation.space = space
      next.activeSessionID = UUID().uuidString
      next.checkpointChat()
      try store.save(next)
      saved = next
      toolActivity = []
      resumed = false
      error = nil
    } catch { self.error = error.localizedDescription }
  }

  func updateProfile(_ profile: AgentProfile) {
    guard !busy, !creatingSpace else { return }
    saved.profile = profile
    resumed = false  // Reapply developer instructions on the next resume.
    persist()
  }

  func persist() {
    saveTask?.cancel()
    guard storageAvailable else { return }
    saved.checkpointChat()
    do { try store.save(saved) } catch {
      self.error = "Could not save the conversation: \(error.localizedDescription)"
    }
  }

  func shutdown() {
    if busy { saved.conversation.lastTurnStatus = "interrupted" }
    persist()
    apiTask?.cancel()
    claudeClient.stop()
    client.stop()
  }

  private func receive(_ method: String, _ params: JSONValue) {
    if method == "account/login/completed" {
      guard loginPending, params["loginId"].string == loginID else { return }
      loginPending = false
      loginID = nil
      if params["success"].bool != true {
        error = params["error"].string ?? "Sign-in did not complete."
      }
      Task { try? await refreshAccount() }
      return
    }
    if method == "account/updated" {
      Task { try? await refreshAccount() }
      return
    }
    if method == "harness/unsupportedRequest" {
      error =
        "The agent requested a tool this first harness does not support. Nothing was approved."
      return
    }
    guard params["threadId"].string == saved.conversation.threadID else { return }
    switch method {
    case "turn/started":
      turnID = params["turn"]["id"].string
    case "item/agentMessage/delta":
      guard let id = params["itemId"].string, let delta = params["delta"].string else { return }
      saved.conversation.appendDelta(itemID: id, text: delta)
      activity = "Replying"
      saveTask?.cancel()
      saveTask = Task { [weak self] in
        do { try await Task.sleep(for: .milliseconds(400)) } catch { return }
        self?.persist()
      }
    case "item/completed":
      let item = params["item"]
      if item["type"].string == "agentMessage", let id = item["id"].string,
        let text = item["text"].string
      {
        saved.conversation.completeMessage(itemID: id, text: text)
        persist()
      }
    case "turn/completed":
      busy = false
      turnID = nil
      activity = ""
      saved.conversation.lastTurnStatus = params["turn"]["status"].string
      if params["turn"]["status"].string == "failed" {
        error = params["turn"]["error"]["message"].string ?? "The reply failed. Try again."
      }
      persist()
    case "error":
      error = params["error"]["message"].string ?? "The agent encountered an error."
    default: break
    }
  }
}
