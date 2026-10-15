import Darwin
import Foundation

public struct CodexInstallation: Sendable {
  public let executable: URL
  public let searchPath: String

  public static func validate(_ path: String) throws -> URL {
    var directory: ObjCBool = false
    guard path.hasPrefix("/"),
      FileManager.default.fileExists(atPath: path, isDirectory: &directory),
      !directory.boolValue, FileManager.default.isExecutableFile(atPath: path)
    else {
      throw HarnessError.message(
        "The selected Codex executable is unavailable: \(path). Choose an executable file or use automatic discovery in Settings."
      )
    }
    return URL(fileURLWithPath: path)
  }

  public static func discover(
    selectedPath: String? = nil,
    environment: [String: String] = ProcessInfo.processInfo.environment,
    home: URL = FileManager.default.homeDirectoryForCurrentUser,
    shellTimeout: TimeInterval = 3,
    fallbackPaths: [String]? = nil
  ) async throws -> CodexInstallation {
    // Validate explicit choices first; a stale choice must not silently launch another CLI.
    let explicit = try
      (selectedPath ?? environment["TONK_CODEX"]
      ?? environment["TONK_TOWN_CODEX"]).map { try validate($0) }
    let inheritedPath = environment["PATH"] ?? "/usr/bin:/bin"
    let shellPath = try await loginShellPath(environment: environment, timeout: shellTimeout)
    let searchPath = [shellPath, inheritedPath].compactMap { $0 }.joined(separator: ":")
    if let explicit { return Self(executable: explicit, searchPath: searchPath) }

    let candidates =
      [inheritedPath, shellPath].compactMap { $0 }.flatMap { path in
        path.split(separator: ":").filter { $0.hasPrefix("/") }.map { "\($0)/codex" }
      }
      + (fallbackPaths ?? [
        "/opt/homebrew/bin/codex", "/usr/local/bin/codex",
        home.appendingPathComponent(".nix-profile/bin/codex").path,
        home.appendingPathComponent(".local/bin/codex").path,
        home.appendingPathComponent(".volta/bin/codex").path,
      ])
    for path in candidates {
      if let executable = try? validate(path) {
        return Self(executable: executable, searchPath: searchPath)
      }
    }
    throw HarnessError.message(
      "Could not find Codex. In Settings, choose your Codex CLI executable, or install the Codex CLI and reconnect."
    )
  }

  // GUI launches do not load shell configuration. Capture only PATH, including
  // interactive version-manager setup, without evaluating any user-supplied text.
  // A file avoids pipe deadlocks when startup scripts print output or spawn children.
  static func loginShellPath(environment: [String: String], timeout: TimeInterval) async throws
    -> String?
  {
    try Task.checkCancellation()
    let shell =
      environment["SHELL"]
      ?? getpwuid(getuid()).flatMap { $0.pointee.pw_shell }.map { String(cString: $0) }
      ?? "/bin/zsh"
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(
      at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
    defer { try? FileManager.default.removeItem(at: directory) }
    let output = directory.appendingPathComponent("path")
    FileManager.default.createFile(
      atPath: output.path, contents: nil, attributes: [.posixPermissions: 0o600])
    let handle = try FileHandle(forWritingTo: output)
    defer { try? handle.close() }
    let process = Process()
    process.executableURL = URL(fileURLWithPath: shell)
    process.arguments = [
      "-ilc", "/usr/bin/printf '\\0TONK_PATH\\0'; /usr/bin/printenv PATH; /usr/bin/printf '\\0'",
    ]
    process.environment = environment
    process.currentDirectoryURL = FileManager.default.temporaryDirectory
    process.standardInput = FileHandle.nullDevice
    process.standardOutput = handle
    process.standardError = FileHandle.nullDevice
    do { try process.run() } catch { return nil }
    defer {
      if process.isRunning { kill(process.processIdentifier, SIGKILL) }
    }
    let deadline = ContinuousClock.now.advanced(by: .seconds(max(0, timeout)))
    while process.isRunning {
      guard ContinuousClock.now < deadline else { return nil }
      try await Task.sleep(for: .milliseconds(20))
    }
    guard process.terminationStatus == 0 else { return nil }
    let text = try String(contentsOf: output, encoding: .utf8)
    guard let marker = text.range(of: "\0TONK_PATH\0", options: .backwards),
      let end = text[marker.upperBound...].firstIndex(of: "\0")
    else { return nil }
    let path = String(text[marker.upperBound..<end]).trimmingCharacters(in: .newlines)
    return path.isEmpty ? nil : path
  }
}
