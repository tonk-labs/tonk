import Foundation
import XCTest

@testable import HarnessCore

final class CodexInstallationTests: XCTestCase {
  private var directory: URL!

  override func setUpWithError() throws {
    directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
  }

  override func tearDownWithError() throws {
    try FileManager.default.removeItem(at: directory)
  }

  private func executable(_ name: String, script: String = "#!/bin/sh\nexit 0\n") throws -> URL {
    let file = directory.appendingPathComponent(name)
    try FileManager.default.createDirectory(
      at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
    try script.write(to: file, atomically: true, encoding: .utf8)
    try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: file.path)
    return file
  }

  func testInheritedPathAndExplicitPrecedence() async throws {
    let installed = try executable("custom npm/bin/codex")
    let chosen = try executable("chosen-codex")
    let override = try executable("override-codex")
    var environment = [
      "PATH": installed.deletingLastPathComponent().path, "SHELL": "/missing-shell",
    ]
    let automatic = try await CodexInstallation.discover(
      environment: environment, fallbackPaths: [])
    XCTAssertEqual(automatic.executable, installed)
    environment["TONK_TOWN_CODEX"] = override.path
    let legacy = try await CodexInstallation.discover(environment: environment, fallbackPaths: [])
    XCTAssertEqual(legacy.executable, override)
    environment["TONK_CODEX"] = chosen.path
    let current = try await CodexInstallation.discover(environment: environment, fallbackPaths: [])
    XCTAssertEqual(current.executable, chosen)
    let selected = try await CodexInstallation.discover(
      selectedPath: installed.path, environment: environment, fallbackPaths: [])
    XCTAssertEqual(selected.executable, installed)
  }

  func testFinderEnvironmentFindsInteractiveLoginShellInstallDespiteStartupOutput() async throws {
    let installed = try executable("node version/bin/codex")
    try "export TONK_TEST_LOGIN=yes\necho login-noise\n".write(
      to: directory.appendingPathComponent(".zprofile"), atomically: true, encoding: .utf8)
    try """
    echo interactive-noise
    if [ "$TONK_TEST_LOGIN" = yes ]; then export PATH="$INSTALL_BIN:/usr/bin:/bin"; fi
    """.write(to: directory.appendingPathComponent(".zshrc"), atomically: true, encoding: .utf8)
    let installation = try await CodexInstallation.discover(
      environment: [
        "PATH": "/usr/bin:/bin", "SHELL": "/bin/zsh", "HOME": directory.path,
        "ZDOTDIR": directory.path, "INSTALL_BIN": installed.deletingLastPathComponent().path,
      ], fallbackPaths: [])
    XCTAssertEqual(installation.executable, installed)
    XCTAssertTrue(
      installation.searchPath.hasPrefix(installed.deletingLastPathComponent().path + ":"))
    XCTAssertFalse(installation.searchPath.contains("noise"))
  }

  func testInvalidExplicitChoiceDoesNotSilentlyFallBack() async throws {
    let installed = try executable("bin/codex")
    let notExecutable = directory.appendingPathComponent("plain-file")
    try "text".write(to: notExecutable, atomically: true, encoding: .utf8)
    for path in [directory.path, notExecutable.path, "/missing-codex", "relative/codex"] {
      do {
        _ = try await CodexInstallation.discover(
          selectedPath: path,
          environment: ["PATH": installed.deletingLastPathComponent().path], fallbackPaths: [])
        XCTFail("Accepted invalid selection: \(path)")
      } catch {
        XCTAssertTrue(error.localizedDescription.contains("Choose an executable file"))
      }
    }
  }

  func testStalledShellIsBoundedAndCommonLocationStillWorks() async throws {
    let shell = try executable("slow-shell", script: "#!/bin/sh\nexec /bin/sleep 30\n")
    let installed = try executable("fallback/codex")
    let started = ContinuousClock.now
    let installation = try await CodexInstallation.discover(
      environment: ["SHELL": shell.path, "PATH": "/missing-bin"], shellTimeout: 0.1,
      fallbackPaths: [installed.path])
    XCTAssertEqual(installation.executable, installed)
    XCTAssertLessThan(started.duration(to: .now), .seconds(2))
  }

  func testMissingInstallHasActionableError() async throws {
    do {
      _ = try await CodexInstallation.discover(
        environment: ["PATH": "/missing-bin", "SHELL": "/missing-shell"], fallbackPaths: [])
      XCTFail("Expected missing installation")
    } catch {
      XCTAssertTrue(error.localizedDescription.contains("In Settings, choose"))
    }
  }

  @MainActor
  func testDiscoveredPathReachesAppServerInterpreter() async throws {
    let interpreter = try executable(
      "runtime bin/fixture-runtime", script: "#!/bin/sh\nexec /usr/bin/python3 \"$@\"\n")
    let agent = try executable(
      "npm bin/codex",
      script: """
        #!/usr/bin/env fixture-runtime
        import json, os, sys
        for line in sys.stdin:
            request = json.loads(line)
            if 'id' in request:
                print(json.dumps({'id':request['id'], 'result':{'path':os.environ['PATH']}}), flush=True)
        """)
    let client = AppServerClient()
    defer { client.stop() }
    let searchPath = interpreter.deletingLastPathComponent().path + ":/usr/bin:/bin"
    try await client.start(
      executable: agent, home: directory.appendingPathComponent("home"), workspace: directory,
      searchPath: searchPath)
    let result = try await client.request("test/path")
    XCTAssertEqual(result["path"].string, agent.deletingLastPathComponent().path + ":" + searchPath)
  }
}
