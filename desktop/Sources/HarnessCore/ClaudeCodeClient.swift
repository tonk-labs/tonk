import Darwin
import Foundation

/// Claude Code owns authentication, conversation sessions, and its agent loop.
@MainActor
public final class ClaudeCodeClient {
  private var process: Process?
  public private(set) var executable: URL?
  private var searchPath = ""
  private let suppliedExecutable: Bool

  public init(executable: URL? = nil) {
    self.executable = executable
    self.suppliedExecutable = executable != nil
  }

  public func discover() async throws {
    if suppliedExecutable { return }
    let environment = ProcessInfo.processInfo.environment
    let shellPath = try await CodexInstallation.loginShellPath(environment: environment, timeout: 3)
    searchPath = [shellPath, environment["PATH"]].compactMap { $0 }.joined(separator: ":")
    let candidates =
      environment["TONK_CLAUDE"].map { [$0] }
      ?? (searchPath.split(separator: ":").filter { $0.hasPrefix("/") }.map { "\($0)/claude" }
        + [
          FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(
            ".local/bin/claude"
          ).path,
          "/opt/homebrew/bin/claude", "/usr/local/bin/claude",
        ])
    executable = candidates.first(where: { FileManager.default.isExecutableFile(atPath: $0) })
      .map { URL(fileURLWithPath: $0) }
    guard executable != nil else {
      throw HarnessError.message(
        "Install Claude Code, then reconnect. Tonk could not find the claude executable.")
    }
  }

  public static func environment(_ source: [String: String]) -> [String: String] {
    var result = source
    for key in source.keys
    where key.hasPrefix("ANTHROPIC_") || key.hasPrefix("CLAUDE_CODE_")
      || key == "CLAUDE_CONFIG_DIR" || key == "CLAUDECODE"
    {
      result.removeValue(forKey: key)
    }
    result["CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"] = "1"
    return result
  }

  public func authenticated(workspace: URL) async throws -> Bool {
    let output = try await run(arguments: ["auth", "status"], workspace: workspace, timeout: 20)
    guard let value = try? JSONDecoder().decode(JSONValue.self, from: output.data) else {
      throw HarnessError.message(
        "Claude Code returned an invalid authentication status. Update Claude Code and reconnect.")
    }
    return value["loggedIn"].bool == true && value["authMethod"].string == "claude.ai"
  }

  public func login(workspace: URL) async throws {
    let result = try await run(
      arguments: ["auth", "login", "--claudeai"], workspace: workspace, timeout: 300)
    guard result.status == 0 else {
      throw HarnessError.message(
        "Claude sign-in did not complete. Run claude auth login in Terminal, then reconnect.")
    }
  }

  public func logout(workspace: URL) async throws {
    let result = try await run(arguments: ["auth", "logout"], workspace: workspace, timeout: 20)
    guard result.status == 0 else {
      throw HarnessError.message("Claude sign-out did not complete. Try again.")
    }
  }

  public func models(workspace: URL) async throws -> [ClaudeModel] {
    let requestID = UUID().uuidString
    let request = JSONValue.object([
      "type": .string("control_request"), "request_id": .string(requestID),
      "request": .object(["subtype": .string("initialize")]),
    ])
    var input = try JSONEncoder().encode(request)
    input.append(10)
    let result = try await run(
      arguments: [
        "-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose",
        "--tools", "", "--disable-slash-commands", "--strict-mcp-config", "--mcp-config",
        "{\"mcpServers\":{}}", "--setting-sources", "", "--settings", "{\"disableAllHooks\":true}",
      ], workspace: workspace, input: input, timeout: 20)
    guard result.status == 0 else {
      throw HarnessError.message("Could not load Claude models. Reconnect to try again.")
    }
    var framer = JSONLines()
    return try Self.decodeModels(framer.append(result.data), requestID: requestID)
  }

  public static func decodeModels(_ events: [JSONValue], requestID: String) throws -> [ClaudeModel]
  {
    guard
      let event = events.first(where: {
        $0["type"].string == "control_response" && $0["response"]["request_id"].string == requestID
      }), event["response"]["subtype"].string == "success",
      case .array(let rows) = event["response"]["response"]["models"]
    else {
      throw HarnessError.message(
        "Claude Code did not return a model list. Update Claude Code and reconnect.")
    }
    var models: [ClaudeModel] = []
    for row in rows {
      guard let id = row["value"].string, !id.isEmpty,
        let name = row["displayName"].string, !name.isEmpty,
        !models.contains(where: { $0.id == id })
      else { continue }
      models.append(ClaudeModel(id: id, name: name, description: row["description"].string ?? ""))
    }
    return models
  }

  public static func arguments(
    session: String, resume: Bool, model: String, mcp: String, instructions: String
  ) -> [String] {
    var args = [
      "-p", "--verbose", "--output-format", "stream-json", "--include-partial-messages",
      "--tools", "", "--disable-slash-commands", "--strict-mcp-config", "--mcp-config", mcp,
      "--setting-sources", "", "--settings", "{\"disableAllHooks\":true}",
      "--permission-mode", "dontAsk", "--allowedTools", "mcp__tonk__*",
      "--max-turns", "24", "--system-prompt", instructions,
      resume ? "--resume" : "--session-id", session,
    ]
    if !model.isEmpty { args += ["--model", model] }
    return args
  }

  public func turn(
    prompt: String, session: String, resume: Bool, model: String,
    workspace: URL, mcp: String, instructions: String, onSession: @escaping () -> Void = {},
    onText: @escaping (String, String) -> Void
  ) async throws {
    var framer = JSONLines()
    var stream = ClaudeCodeStream()
    let result = try await run(
      arguments: Self.arguments(
        session: session, resume: resume, model: model, mcp: mcp, instructions: instructions),
      workspace: workspace, input: Data(prompt.utf8), timeout: 1800
    ) { data in
      for event in try framer.append(data) {
        if let text = try stream.receive(event) { onText(text.0, text.1) }
        if event["type"].string == "assistant", event["error"] == .null { onSession() }
      }
    }
    guard result.status == 0, stream.completed else {
      throw HarnessError.message(
        "Claude Code stopped before completing the turn. Reconnect and try again.")
    }
  }

  public func stop() {
    if let process, process.isRunning { process.terminate() }
  }

  private func run(
    arguments: [String], workspace: URL, input: Data? = nil, timeout: TimeInterval,
    onData: ((Data) throws -> Void)? = nil
  ) async throws -> (status: Int32, data: Data) {
    guard process == nil, let executable else {
      throw HarnessError.message("Claude Code is unavailable or already running.")
    }
    try FileManager.default.createDirectory(
      at: workspace, withIntermediateDirectories: true,
      attributes: [.posixPermissions: 0o700])
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(
      at: directory, withIntermediateDirectories: true,
      attributes: [.posixPermissions: 0o700])
    defer { try? FileManager.default.removeItem(at: directory) }
    let output = directory.appendingPathComponent("output")
    FileManager.default.createFile(
      atPath: output.path, contents: nil, attributes: [.posixPermissions: 0o600])
    let writer = try FileHandle(forWritingTo: output)
    let reader = try FileHandle(forReadingFrom: output)
    defer {
      try? writer.close()
      try? reader.close()
    }
    let child = Process()
    child.executableURL = executable
    child.arguments = arguments
    var env = Self.environment(ProcessInfo.processInfo.environment)
    env["PATH"] = executable.deletingLastPathComponent().path + ":" + searchPath
    child.environment = env
    child.currentDirectoryURL = workspace
    child.standardOutput = writer
    child.standardError = FileHandle.nullDevice
    if let input {
      let inputURL = directory.appendingPathComponent("input")
      try input.write(to: inputURL, options: .atomic)
      try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: inputURL.path)
      child.standardInput = try FileHandle(forReadingFrom: inputURL)
    } else {
      child.standardInput = FileHandle.nullDevice
    }
    try child.run()
    process = child
    defer {
      if child.isRunning { kill(child.processIdentifier, SIGKILL) }
      if input != nil { try? (child.standardInput as? FileHandle)?.close() }
      process = nil
    }
    var collected = Data()
    let deadline = Date().addingTimeInterval(timeout)
    repeat {
      try Task.checkCancellation()
      guard Date() < deadline else { throw HarnessError.message("Claude Code timed out.") }
      let bytes = reader.availableData
      if !bytes.isEmpty {
        if let onData {
          try onData(bytes)
        } else {
          guard collected.count + bytes.count <= 1_048_576 else {
            throw HarnessError.message("Claude Code output exceeded its limit.")
          }
          collected.append(bytes)
        }
      }
      if !child.isRunning {
        let tail = reader.readDataToEndOfFile()
        if let onData { try onData(tail) } else { collected.append(tail) }
        break
      }
      try await Task.sleep(for: .milliseconds(30))
    } while true
    return (child.terminationStatus, collected)
  }
}

public struct ClaudeModel: Identifiable, Equatable, Sendable {
  public let id: String
  public let name: String
  public let description: String
}

public struct ClaudeCodeStream {
  public private(set) var completed = false
  private var messageID = UUID().uuidString
  private var streamed = false
  public init() {}
  public mutating func receive(_ value: JSONValue) throws -> (String, String)? {
    if value["type"].string == "stream_event" {
      let event = value["event"]
      if event["type"].string == "message_start" {
        messageID = event["message"]["id"].string ?? UUID().uuidString
        streamed = false
      }
      if event["delta"]["type"].string == "text_delta", let text = event["delta"]["text"].string {
        streamed = true
        return (messageID, text)
      }
    }
    if value["error"].string == "authentication_failed" {
      throw ClaudeCodeError.authentication
    }
    if value["type"].string == "assistant", !streamed {
      let text = value["message"]["content"].array.compactMap { $0["text"].string }.joined()
      if !text.isEmpty { return (value["message"]["id"].string ?? messageID, text) }
    }
    if value["type"].string == "result" {
      guard value["is_error"].bool != true, value["subtype"].string == "success" else {
        let detail =
          value["errors"].array.compactMap(\.string).joined(separator: "\n").nilIfEmpty
          ?? value["result"].string ?? "Claude Code could not complete this turn."
        if detail.contains("Failed to authenticate") || detail.contains("OAuth session expired") {
          throw ClaudeCodeError.authentication
        }
        throw HarnessError.message(detail)
      }
      completed = true
    }
    return nil
  }
}

extension String { fileprivate var nilIfEmpty: String? { isEmpty ? nil : self } }

public enum ClaudeCodeError: LocalizedError {
  case authentication
  public var errorDescription: String? {
    "Your Claude login has expired. Sign in with Claude again to continue."
  }
}
