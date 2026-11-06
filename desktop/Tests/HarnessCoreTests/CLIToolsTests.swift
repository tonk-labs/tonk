import XCTest

@testable import HarnessCore

final class CLIToolsTests: XCTestCase {
  func testOnlyFixedOperationsAndTargetsAreAccepted() throws {
    XCTAssertEqual(
      try CLITools.arguments(
        .object(["operation": .string("query"), "target": .string("tonk-checklist")])),
      ["--space", "attached", "query", "tonk-checklist"])
    for value: JSONValue in [
      .object(["operation": .string("join"), "target": .string("secret")]),
      .object(["operation": .string("query"), "target": .string("--help")]),
      .object(["operation": .string("query"), "target": .string("task"), "space": .string("other")]
      ),
      .object(["operation": .string("guide"), "target": .string("invite")]),
      .object([
        "operation": .string("apply"),
        "document": .string("task!:\n  title: !include file:///private/key"),
      ]),
      .object([
        "operation": .string("apply"),
        "document": .string("%TAG !e! tag:yaml.org,2002:\n---\ntask!:\n  title: !e!include secret"),
      ]),
      .object([
        "operation": .string("apply"), "document": .string("task!:\n  title: !<include> secret"),
      ]),
      .object([
        "operation": .string("apply"), "document": .string(String(repeating: "x", count: 32001)),
      ]),
    ] { XCTAssertThrowsError(try CLITools.arguments(value)) }
  }

  func testPreviewAndApplyUseInlineDocumentAndFixedSyncPolicy() throws {
    let doc = "tonk-checklist!:\n  this: id:tonk-checklist-smoke\n  verify: Done"
    let preview = try CLITools.arguments(
      .object([
        "operation": .string("preview"), "document": .string(doc),
        "home": .string("tonk-checklist"),
      ]))
    XCTAssertEqual(
      preview,
      [
        "--space", "attached", "eval", "-c", doc, "--quiet", "--json", "--home", "tonk-checklist",
        "--dry-run",
      ])
    let apply = try CLITools.arguments(
      .object(["operation": .string("apply"), "document": .string(doc)]))
    XCTAssertFalse(apply.contains("--dry-run"))
    XCTAssertFalse(apply.contains("--no-sync"))
  }

  func testSummaryKeepsCommitOutcomeAndWarnings() {
    let output =
      "warning: account directory update failed\n{\"revision_before\":1,\"revision_after\":2,\"commits\":{\"claims\":1}}\n"
    let result = CLITools.summarize(output)
    XCTAssertTrue(result.contains("warning: account directory update failed"))
    XCTAssertTrue(result.contains("\"claims\":1"))
    XCTAssertTrue(result.contains("\"revisionChanged\":true"))
    XCTAssertFalse(result.contains("revision_before"))
  }
}
