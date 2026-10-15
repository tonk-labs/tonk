import Foundation
import XCTest

@testable import HarnessCore

@MainActor
final class TonkCLITests: XCTestCase {
  func testToolReadPullsFirstAndDoesNotReadAfterFailedPull() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    let cli = TonkCLI(root: root, subject: "did:key:zScratch")
    try FileManager.default.createDirectory(at: cli.state, withIntermediateDirectories: true)
    let registry =
      "{\"spaces\":{\"attached\":{\"connection\":{\"subject\":\"did:key:zScratch\",\"version\":1,\"recipient\":\"did:key:zTool\"}}}}"
    try Data(registry.utf8).write(to: cli.state.appendingPathComponent("spaces.json"))
    let script = root.appendingPathComponent("fixture")
    try Data(
      """
      #!/bin/sh
      printf '%s\\n' "$3" >> "$TONK_SPACES_STATE/calls"
      if [ "$3" = pull ]; then
        [ ! -f "$TONK_SPACES_STATE/fail" ] || exit 1
        printf current > "$TONK_SPACES_STATE/record"
      else
        cat "$TONK_SPACES_STATE/record"
      fi
      """.utf8
    ).write(to: script)
    try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: script.path)
    let old = ProcessInfo.processInfo.environment["TONK_CLI"]
    setenv("TONK_CLI", script.path, 1)
    defer { if let old { setenv("TONK_CLI", old, 1) } else { unsetenv("TONK_CLI") } }
    for operation in ["query", "show"] {
      let request: JSONValue = .object(["operation": .string(operation), "target": .string("task")])
      let result = try await cli.executeTool(request)
      XCTAssertEqual(result, "current")
    }
    try Data().write(to: cli.state.appendingPathComponent("fail"))
    do {
      _ = try await cli.executeTool(
        .object(["operation": .string("query"), "target": .string("task")]))
      XCTFail("Must not report stale data after a failed pull")
    } catch {}
    let calls = try String(contentsOf: cli.state.appendingPathComponent("calls"), encoding: .utf8)
    XCTAssertEqual(calls, "pull\nquery\npull\nshow\npull\n")
  }

  func testAutomaticSetupImportsOnceAndReusesConnection() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let script = root.appendingPathComponent("fixture")
    let registry =
      "{\"spaces\":{\"attached\":{\"connection\":{\"subject\":\"did:key:zScratch\",\"version\":1,\"recipient\":\"did:key:zTool\"}}}}"
    try Data("#!/bin/sh\nprintf '%s' '\(registry)' > \"$TONK_SPACES_STATE/spaces.json\"\n".utf8)
      .write(to: script)
    try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: script.path)
    let old = ProcessInfo.processInfo.environment["TONK_CLI"]
    setenv("TONK_CLI", script.path, 1)
    defer { if let old { setenv("TONK_CLI", old, 1) } else { unsetenv("TONK_CLI") } }
    let cli = TonkCLI(root: root, subject: "did:key:zScratch")
    var invitations = 0
    for _ in 0..<2 {
      try await cli.ensureConnected {
        invitations += 1
        return "private fixture invitation"
      }
    }
    XCTAssertEqual(invitations, 1)
    XCTAssertTrue(cli.isConnected)
    XCTAssertNil(cli.pendingLink)
  }

  func testAutomaticSetupResumesRetainedInvitation() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    let cli = TonkCLI(root: root, subject: "did:key:zScratch")
    try FileManager.default.createDirectory(at: cli.directory, withIntermediateDirectories: true)
    try Data("retained fixture invitation".utf8).write(
      to: cli.directory.appendingPathComponent("pending-link"))
    let old = ProcessInfo.processInfo.environment["TONK_CLI"]
    setenv("TONK_CLI", "/usr/bin/false", 1)
    defer { if let old { setenv("TONK_CLI", old, 1) } else { unsetenv("TONK_CLI") } }
    var invitations = 0
    do {
      try await cli.ensureConnected {
        invitations += 1
        return "new invitation"
      }
      XCTFail("Expected fixture import failure")
    } catch {}
    XCTAssertEqual(invitations, 0)
    XCTAssertEqual(cli.pendingLink, "retained fixture invitation")
  }

  func testBindingRejectsWrongSubjectAndOrdinaryAccountReplica() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    let cli = TonkCLI(root: root, subject: "did:key:zScratch")
    let other = TonkCLI(root: root, subject: "did:key:zOther")
    XCTAssertNotEqual(cli.state, other.state)
    XCTAssertFalse(cli.isConnected)
    try FileManager.default.createDirectory(at: cli.state, withIntermediateDirectories: true)
    for connection: [String: Any] in [
      [:], ["subject": "did:key:zOther", "version": 1, "recipient": "did:key:zTool"],
    ] {
      let data = try JSONSerialization.data(withJSONObject: [
        "spaces": ["attached": ["connection": connection]]
      ])
      try data.write(to: cli.state.appendingPathComponent("spaces.json"))
      XCTAssertThrowsError(try cli.verifyBinding())
    }
    let data = Data(
      "{\"spaces\":{\"attached\":{\"connection\":{\"subject\":\"did:key:zScratch\",\"version\":1,\"recipient\":\"did:key:zTool\"}}}}"
        .utf8)
    try data.write(to: cli.state.appendingPathComponent("spaces.json"))
    XCTAssertTrue(cli.isConnected)
  }

  func testProcessUsesIsolatedStateAndDoesNotPassAmbientSpace() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let script = root.appendingPathComponent("fixture")
    try Data(
      "#!/bin/sh\nprintf '%s\\n' \"$TONK_SPACES_STATE\" \"${TONK_SPACE-unset}\" \"$1\" \"$TONK_CONNECTION_ORIGIN\"\n"
        .utf8
    ).write(to: script)
    try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: script.path)
    let oldBinary = ProcessInfo.processInfo.environment["TONK_CLI"]
    let oldSpace = ProcessInfo.processInfo.environment["TONK_SPACE"]
    setenv("TONK_CLI", script.path, 1)
    setenv("TONK_SPACE", "ambient", 1)
    defer {
      if let oldBinary {
        setenv("TONK_CLI", oldBinary, 1)
      } else {
        unsetenv("TONK_CLI")
      }
      if let oldSpace { setenv("TONK_SPACE", oldSpace, 1) } else { unsetenv("TONK_SPACE") }
    }
    let cli = TonkCLI(root: root, subject: "did:key:zScratch")
    let output = try await cli.run(["literal; no shell evaluation"])
    XCTAssertEqual(
      output,
      "\(cli.state.path)\nunset\nliteral; no shell evaluation\n\(RuntimeLocation.home.absoluteString)\n"
    )
    let privateResult = try await cli.run(["private invitation"], privateOutput: true)
    XCTAssertFalse(privateResult.contains("private invitation"))
  }
  func testCancellationStopsProcessAndRemovesPrivateOutput() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let script = root.appendingPathComponent("fixture")
    try Data("#!/usr/bin/python3\nimport time\ntime.sleep(30)\n".utf8).write(to: script)
    try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: script.path)
    let old = ProcessInfo.processInfo.environment["TONK_CLI"]
    setenv("TONK_CLI", script.path, 1)
    defer { if let old { setenv("TONK_CLI", old, 1) } else { unsetenv("TONK_CLI") } }
    let cli = TonkCLI(root: root, subject: "did:key:zScratch")
    let task = Task { try await cli.run([]) }
    try await Task.sleep(for: .milliseconds(200))
    task.cancel()
    do {
      _ = try await task.value
      XCTFail("Expected cancellation")
    } catch { XCTAssertTrue(error is CancellationError) }
    let names = try FileManager.default.contentsOfDirectory(atPath: cli.directory.path)
    XCTAssertFalse(names.contains(where: { $0.hasPrefix("run-") }))
  }

}
