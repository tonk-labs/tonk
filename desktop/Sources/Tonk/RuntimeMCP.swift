import Foundation
import HarnessCore

@MainActor
extension RuntimeModel {
  /// Opt-in launch argument pins this process's external tools to one space.
  func startMCPBridge(root: URL) async {
    let arguments = ProcessInfo.processInfo.arguments
    guard mcpBridge == nil, let index = arguments.firstIndex(of: "--mcp-space"),
      arguments.indices.contains(index + 1)
    else { return }
    let subject = arguments[index + 1]
    let bridge = LocalRuntimeBridge(
      allowsWrites: arguments.contains("--mcp-write"), supportsInspection: true
    ) {
      [weak self] name, arguments in
      guard let self, let space = self.spaces.first(where: { $0.subject == subject }) else {
        throw HarnessError.message("The configured MCP space is unavailable in this account.")
      }
      return try await self.performBuildTool(space, tool: name, arguments: arguments)
    }
    do {
      let descriptor = try await bridge.start()
      let directory = root.appendingPathComponent("MCP", isDirectory: true)
      try FileManager.default.createDirectory(
        at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
      try FileManager.default.setAttributes(
        [.posixPermissions: 0o700], ofItemAtPath: directory.path)
      let file = directory.appendingPathComponent("connection.json")
      let data = try JSONEncoder().encode(descriptor)
      try data.write(to: file, options: .atomic)
      try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)
      mcpBridge = bridge
      mcpConnectionFile = file
    } catch {
      bridge.stop()
      self.error = "Could not start the local MCP connection: " + error.localizedDescription
    }
  }

  func stopMCPBridge() {
    mcpBridge?.stop()
    mcpBridge = nil
    if let file = mcpConnectionFile { try? FileManager.default.removeItem(at: file) }
    mcpConnectionFile = nil
  }
}
