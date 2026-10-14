import Foundation
import XCTest

@testable import HarnessCore

final class SpaceToolsTests: XCTestCase {
  func testAgentToolEnvelopesUseSharedPrimitivesAndNativeControls() {
    for includeCLI in [false, true] {
      let tools = SpaceTools.agentDefinitions(includeCLI: includeCLI)
      XCTAssertEqual(
        tools.compactMap { $0["name"].string },
        ["tonk_query", "tonk_evaluate", "tonk_guide", "tonk_inspect_view", "tonk_propose_space"])
      for tool in tools {
        XCTAssertEqual(tool["type"].string, "function")
        XCTAssertNotNil(tool["name"].string)
        XCTAssertNotNil(tool["description"].string)
        XCTAssertEqual(tool["inputSchema"]["type"].string, "object")
        XCTAssertEqual(tool["annotations"], .null)
      }
    }
  }

  func testRejectsTargetOverridesAndInvalidWrites() throws {
    XCTAssertNil(try SpaceTools.validate(tool: "tonk_inspect_view", arguments: .object([:])))
    for key in ["space", "url", "script", "selector"] {
      XCTAssertThrowsError(
        try SpaceTools.validate(
          tool: "tonk_inspect_view",
          arguments: .object([key: .string("override")])))
    }
    for tool in ["tonk_space_info", "tonk_space_schema", "tonk_rename_space", "tonk_apply", "tonk_preview", "tonk_cli"] {
      XCTAssertThrowsError(try SpaceTools.validate(tool: tool, arguments: .object([:])))
    }
    XCTAssertThrowsError(try SpaceTools.validate(tool: "shell", arguments: .object([:])))
  }

  @MainActor
  func testReadbackWaitsForEffectsAndStopsWithoutResubmitting() async throws {
    var reads = 0
    let verified = try await SpaceTools.waitForName(
      "New",
      read: {
        reads += 1
        return reads < 3 ? "Old" : "New"
      }, pause: {})
    XCTAssertTrue(verified)
    XCTAssertEqual(reads, 3)
    reads = 0
    let missing = try await SpaceTools.waitForName(
      "New",
      read: {
        reads += 1
        return "Old"
      }, pause: {})
    XCTAssertFalse(missing)
    XCTAssertEqual(reads, 10)
    do {
      _ = try await SpaceTools.waitForName(
        "New", read: { "Old" }, pause: { throw CancellationError() })
      XCTFail("Cancellation must stop verification")
    } catch { XCTAssertTrue(error is CancellationError) }
  }

  func testAttachmentPersistsAndOldConversationsRemainReadable() throws {
    let space = try XCTUnwrap(
      TonkSpace.decodeCatalog(Data("[{\"subject\":\"did:key:z123\",\"name\":\"Scratch\"}]".utf8))
        .first)
    var conversation = Conversation()
    conversation.space = space
    XCTAssertEqual(
      try JSONDecoder().decode(Conversation.self, from: JSONEncoder().encode(conversation)).space,
      space)
    XCTAssertNil(
      try JSONDecoder().decode(Conversation.self, from: Data("{\"messages\":[]}".utf8)).space)
  }
}
