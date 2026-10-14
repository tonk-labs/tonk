import Foundation

/// A small provider boundary. Credentials remain entirely inside Codex.
@MainActor
public final class AppServerClient {
  public var onNotification: ((String, JSONValue) -> Void)?
  public var onToolCall: ((JSONValue) async -> JSONValue)?
  private var toolTasks: [UUID: Task<Void, Never>] = [:]
  public var onDisconnect: ((String) -> Void)?
  private var process: Process?
  private var input: FileHandle?
  private var pending: [Int: CheckedContinuation<JSONValue, Error>] = [:]
  private var timeouts: [Int: Task<Void, Never>] = [:]
  private var nextID = 0
  private var generation = UUID()
  private var framer = JSONLines()

  public init() {}

  public func start(executable: URL, home: URL, workspace: URL, searchPath: String? = nil)
    async throws
  {
    guard process == nil else { return }
    try FileManager.default.createDirectory(
      at: home, withIntermediateDirectories: true,
      attributes: [.posixPermissions: 0o700])
    try FileManager.default.createDirectory(
      at: workspace, withIntermediateDirectories: true,
      attributes: [.posixPermissions: 0o700])
    let child = Process()
    child.executableURL = executable
    child.arguments = [
      "app-server", "--stdio",
      "-c", "features.shell_tool=false", "-c", "features.multi_agent=false",
      "-c", "web_search=\"disabled\"",
      "-c", "cli_auth_credentials_store=\"file\"",
    ]
    var environment = ProcessInfo.processInfo.environment
    environment["CODEX_HOME"] = home.path
    // Finder's PATH is smaller than a login shell's. No shell interpolation.
    environment["PATH"] = [
      executable.deletingLastPathComponent().path,
      searchPath ?? environment["PATH"] ?? "/usr/bin:/bin",
    ].joined(separator: ":")
    environment.removeValue(forKey: "OPENAI_API_KEY")
    environment.removeValue(forKey: "CODEX_API_KEY")
    environment.removeValue(forKey: "CODEX_ACCESS_TOKEN")
    child.environment = environment
    child.currentDirectoryURL = workspace
    let stdin = Pipe()
    let stdout = Pipe()
    child.standardInput = stdin
    child.standardOutput = stdout
    child.standardError = FileHandle.nullDevice
    let token = UUID()
    generation = token
    framer = JSONLines()
    try child.run()
    process = child
    input = stdin.fileHandleForWriting
    let reader = stdout.fileHandleForReading
    Task.detached { [weak self] in
      while true {
        let data = reader.availableData
        if data.isEmpty { break }
        await self?.receive(data, generation: token)
      }
      child.waitUntilExit()
      await self?.didExit(generation: token, status: child.terminationStatus)
    }
    do {
      _ = try await request(
        "initialize",
        params: .object([
          "capabilities": .object(["experimentalApi": .bool(true)]),
          "clientInfo": .object([
            "name": .string("tonk"),
            "title": .string("Tonk"), "version": .string("0.1.0"),
          ]),
        ]))
      try write(.object(["method": .string("initialized")]))
    } catch {
      stop()
      throw error
    }
  }

  public func request(
    _ method: String, params: JSONValue = .object([:]),
    timeout: TimeInterval = 30
  ) async throws -> JSONValue {
    guard process?.isRunning == true else {
      throw HarnessError.message("The agent is disconnected. Reconnect to continue.")
    }
    nextID += 1
    let id = nextID
    return try await withCheckedThrowingContinuation { continuation in
      pending[id] = continuation
      do {
        try write(
          .object(["id": .number(Double(id)), "method": .string(method), "params": params]))
        timeouts[id] = Task { [weak self] in
          do { try await Task.sleep(for: .seconds(timeout)) } catch { return }
          self?.finish(
            id,
            with: .failure(
              HarnessError.message("The agent timed out during \(method). Reconnect and try again.")
            ))
        }
      } catch { finish(id, with: .failure(error)) }
    }
  }

  public func stop() {
    generation = UUID()
    cancelTools()
    try? input?.close()
    input = nil
    if process?.isRunning == true { process?.terminate() }
    process = nil
    failPending("Agent connection closed.")
  }

  public func cancelTools() {
    for task in toolTasks.values { task.cancel() }
    toolTasks.removeAll()
  }

  private func write(_ value: JSONValue) throws {
    guard let input else { throw HarnessError.message("Agent connection closed.") }
    var data = try JSONEncoder().encode(value)
    data.append(10)
    try input.write(contentsOf: data)
  }

  private func receive(_ data: Data, generation token: UUID) {
    guard token == generation else { return }
    do {
      for message in try framer.append(data) {
        if let method = message["method"].string {
          if message["id"] != .null {
            if method == "item/tool/call", let handler = onToolCall {
              let taskID = UUID()
              let requestID = message["id"]
              toolTasks[taskID] = Task { [weak self] in
                let result = await handler(message["params"])
                guard let self else { return }
                self.toolTasks.removeValue(forKey: taskID)
                guard !Task.isCancelled, token == self.generation else { return }
                try? self.write(.object(["id": requestID, "result": result]))
              }
              continue
            }
            // Native tools do not grant shell or file-change approvals.
            if method == "item/commandExecution/requestApproval"
              || method == "item/fileChange/requestApproval"
            {
              try write(
                .object(["id": message["id"], "result": .object(["decision": .string("decline")])]))
            } else {
              try write(
                .object([
                  "id": message["id"],
                  "error": .object([
                    "code": .number(-32601),
                    "message": .string("This harness does not support this request."),
                  ]),
                ]))
            }
            onNotification?("harness/unsupportedRequest", .object(["method": .string(method)]))
          } else {
            onNotification?(method, message["params"])
          }
        } else if let id = message["id"].int {
          if message["error"] != .null {
            finish(
              id,
              with: .failure(
                HarnessError.message(message["error"]["message"].string ?? "Agent request failed."))
            )
          } else {
            finish(id, with: .success(message["result"]))
          }
        }
      }
    } catch {
      stop()
      onDisconnect?("The agent sent an unreadable response. Reconnect to continue.")
    }
  }

  private func finish(_ id: Int, with result: Result<JSONValue, Error>) {
    timeouts.removeValue(forKey: id)?.cancel()
    pending.removeValue(forKey: id)?.resume(with: result)
  }

  private func failPending(_ message: String) {
    for id in Array(pending.keys) { finish(id, with: .failure(HarnessError.message(message))) }
  }

  private func didExit(generation token: UUID, status: Int32) {
    guard token == generation else { return }
    cancelTools()
    generation = UUID()
    input = nil
    process = nil
    let message = "Agent process exited (\(status)). Reconnect to continue."
    failPending(message)
    onDisconnect?(message)
  }
}
