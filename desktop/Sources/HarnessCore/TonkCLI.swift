import CryptoKit
import Darwin
import Foundation

@MainActor
public final class TonkCLI {
  public let directory: URL
  public let state: URL
  public let workspace: URL
  public let subject: String
  private var running = false
  private var preparing = false
  private var usingTool = false

  public init(root: URL, subject: String) {
    self.subject = subject
    let key = SHA256.hash(data: Data(subject.utf8)).map { String(format: "%02x", $0) }.joined()
    directory = root.appendingPathComponent("CLI/\(key)", isDirectory: true)
    state = directory.appendingPathComponent("State", isDirectory: true)
    workspace = directory.appendingPathComponent("Workspace", isDirectory: true)
  }

  public var isConnected: Bool { (try? verifyBinding()) != nil }

  public func verifyBinding() throws {
    let data = try Data(contentsOf: state.appendingPathComponent("spaces.json"))
    let registry = try JSONDecoder().decode(JSONValue.self, from: data)
    guard registry["spaces"]["attached"]["connection"]["subject"].string == subject,
      registry["spaces"]["attached"]["connection"]["version"] == .number(1),
      registry["spaces"]["attached"]["connection"]["recipient"].string?.hasPrefix("did:key:")
        == true
    else { throw HarnessError.message("The CLI replica does not match this attached space.") }
  }

  public var pendingLink: String? {
    try? String(contentsOf: directory.appendingPathComponent("pending-link"), encoding: .utf8)
  }

  public func ensureConnected(createLink: () async throws -> String) async throws {
    guard !preparing else {
      throw HarnessError.message(
        "This space is still connecting. Retry the tool when setup finishes.")
    }
    preparing = true
    defer { preparing = false }
    try Task.checkCancellation()
    if pendingLink != nil || !isConnected {
      let link: String
      if let pending = pendingLink { link = pending } else { link = try await createLink() }
      try Task.checkCancellation()
      try await connect(link: link)
    }
    try verifyBinding()
  }

  public func connect(link: String) async throws {
    try FileManager.default.createDirectory(
      at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
    let pending = directory.appendingPathComponent("pending-link")
    if !FileManager.default.fileExists(atPath: pending.path) {
      // Retain exactly this invitation for retry instead of issuing more identities.
      try Data(link.utf8).write(to: pending, options: [.atomic])
      try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: pending.path)
    }
    let retained = try String(contentsOf: pending, encoding: .utf8)
    _ = try await run(["join", retained, "--name", "attached"], privateOutput: true)
    try verifyBinding()
    try FileManager.default.removeItem(at: pending)
  }

  /// Read the shared state before returning records or schema to the model.
  public func executeTool(_ value: JSONValue) async throws -> String {
    let arguments = try CLITools.arguments(value)
    guard !usingTool else { throw HarnessError.message("A CLI tool is already running.") }
    usingTool = true
    defer { usingTool = false }
    try verifyBinding()
    if value["operation"].string == "query" || value["operation"].string == "show" {
      _ = try await run(["--space", "attached", "pull"])
      try Task.checkCancellation()
    }
    return try await run(arguments)
  }

  public func status() async throws -> String {
    try verifyBinding()
    return try await run(["--space", "attached", "status", "--json"])
  }

  public func run(_ arguments: [String], privateOutput: Bool = false) async throws -> String {
    guard !running else { throw HarnessError.message("A CLI operation is already running.") }
    running = true
    defer { running = false }
    let fm = FileManager.default
    for path in [directory, state, workspace] {
      try fm.createDirectory(
        at: path, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
    }
    let candidates = [
      ProcessInfo.processInfo.environment["TONK_CLI"],
      fm.homeDirectoryForCurrentUser.appendingPathComponent(".cargo/bin/tonk").path,
      "/opt/homebrew/bin/tonk", "/usr/local/bin/tonk",
    ].compactMap { $0 }
    guard let binary = candidates.first(where: { fm.isExecutableFile(atPath: $0) }) else {
      throw HarnessError.message("Install the Tonk CLI or set TONK_CLI to its executable.")
    }
    let output = directory.appendingPathComponent("run-\(UUID().uuidString).log")
    fm.createFile(atPath: output.path, contents: nil, attributes: [.posixPermissions: 0o600])
    let handle = try FileHandle(forWritingTo: output)
    defer {
      try? handle.close()
      try? fm.removeItem(at: output)
    }
    let process = Process()
    process.executableURL = URL(fileURLWithPath: binary)
    process.arguments = arguments
    process.currentDirectoryURL = workspace
    var environment: [String: String] = [:]
    for key in ["HOME", "PATH", "TMPDIR", "LANG", "SSL_CERT_FILE", "SSL_CERT_DIR"] {
      environment[key] = ProcessInfo.processInfo.environment[key]
    }
    environment["TONK_SPACES_STATE"] = state.path
    environment["TONK_CONNECTION_ORIGIN"] = RuntimeLocation.home.absoluteString
    environment["TONK_TELEMETRY"] = "0"
    environment["TONK_NO_UPDATE_CHECK"] = "1"
    process.environment = environment
    process.standardInput = FileHandle.nullDevice
    process.standardOutput = handle
    process.standardError = handle
    try process.run()
    defer { if process.isRunning { kill(process.processIdentifier, SIGKILL) } }
    let deadline = Date().addingTimeInterval(120)
    while process.isRunning {
      try Task.checkCancellation()
      guard Date() < deadline else {
        throw HarnessError.message(
          "The CLI operation timed out; check its state before retrying a write.")
      }
      let size = (try? fm.attributesOfItem(atPath: output.path)[.size] as? NSNumber)?.intValue ?? 0
      guard size <= 1_000_000 else {
        throw HarnessError.message("CLI output exceeded the harness limit.")
      }
      try await Task.sleep(for: .milliseconds(100))
    }
    guard process.terminationStatus == 0 else {
      throw HarnessError.message(
        "Tonk CLI failed (exit \(process.terminationStatus)). Private command output was not added to chat."
      )
    }
    if privateOutput { return "CLI connection imported." }
    let data = try Data(contentsOf: output)
    guard data.count <= 1_000_000 else {
      throw HarnessError.message("CLI output exceeded the harness limit.")
    }
    return String(decoding: data, as: UTF8.self)
  }
}
