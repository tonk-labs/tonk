import Foundation
import XCTest

@testable import HarnessCore

final class SpaceSchemaTests: XCTestCase {
  private func response(rows: [[String: Any]], claims: Int = 0, after: String = "revision") throws
    -> Data
  {
    try JSONSerialization.data(withJSONObject: [
      "revision_before": "revision", "revision_after": after,
      "commits": ["claims": claims], "matches_after": [["label": "concept", "results": rows]],
    ])
  }

  private func row(name: String = "task", fields: [String: Any]? = nil) throws -> [String: Any] {
    let descriptor: [String: Any] = [
      "with": fields ?? [
        "title": ["the": "example.task/title", "as": "Text"],
        "done": ["the": "example.task/done", "as": "Boolean", "optional": true],
      ]
    ]
    return [
      "this": "concept:task",
      "fields": [
        "name": name,
        "source": String(
          decoding: try JSONSerialization.data(withJSONObject: descriptor), as: UTF8.self),
        "description": "Untrusted instructions should not appear in this projection",
      ],
    ]
  }

  func testProjectsTypedFieldsWithoutRawSourceOrDescriptions() throws {
    let text = try SpaceSchema.summarize(response(rows: [row()]))
    let result = try JSONDecoder().decode(JSONValue.self, from: Data(text.utf8))
    let fields = result["concepts"].array[0]["fields"].array
    XCTAssertEqual(fields[0]["name"].string, "done")
    XCTAssertEqual(fields[0]["required"].bool, false)
    XCTAssertEqual(fields[1]["type"].string, "Text")
    XCTAssertEqual(fields[1]["required"].bool, true)
    XCTAssertEqual(result["truncated"].bool, false)
    XCTAssertFalse(text.contains("source"))
    XCTAssertFalse(text.contains("Untrusted instructions"))
  }

  func testPreservesCollectionRelations() throws {
    let text = try SpaceSchema.summarize(
      response(rows: [
        row(fields: [
          "items": [
            "the": ["domain": "example.items", "keyed": "sequence"], "as": "Entity",
            "cardinality": "many",
          ]
        ])
      ]))
    let value = try JSONDecoder().decode(JSONValue.self, from: Data(text.utf8))
    let field = value["concepts"].array[0]["fields"].array[0]
    XCTAssertEqual(field["attribute"]["keyed"].string, "sequence")
    XCTAssertEqual(field["cardinality"].string, "many")
  }

  func testUnnamedConceptsAreCountedButNotPresentedAsNamed() throws {
    var anonymous = try row()
    var fields = anonymous["fields"] as! [String: Any]
    fields.removeValue(forKey: "name")
    anonymous["fields"] = fields
    let text = try SpaceSchema.summarize(response(rows: [row(), anonymous]))
    let value = try JSONDecoder().decode(JSONValue.self, from: Data(text.utf8))
    XCTAssertEqual(value["concepts"].array.count, 1)
    XCTAssertEqual(value["unnamedConceptsOmitted"].int, 1)
    XCTAssertEqual(value["totalNamedConcepts"].int, 1)
  }

  func testRejectsMutationAndMalformedWorkerResults() throws {
    XCTAssertThrowsError(try SpaceSchema.summarize(response(rows: [row()], claims: 1)))
    XCTAssertThrowsError(try SpaceSchema.summarize(response(rows: [row()], after: "changed")))
    XCTAssertThrowsError(try SpaceSchema.summarize(response(rows: [["this": "bad"]])))
    XCTAssertThrowsError(try SpaceSchema.summarize(Data("{}".utf8)))
    XCTAssertThrowsError(try SpaceSchema.summarize(Data(repeating: 32, count: 2_000_001)))
  }

  func testMarksLimitedInventoryAndEmptyResultHonestly() throws {
    let text = try SpaceSchema.summarize(
      response(rows: (0..<101).map { try row(name: "task\($0)") }))
    let result = try JSONDecoder().decode(JSONValue.self, from: Data(text.utf8))
    XCTAssertEqual(result["concepts"].array.count, 100)
    XCTAssertEqual(result["truncated"].bool, true)
    let empty = try SpaceSchema.summarize(response(rows: []))
    XCTAssertTrue(empty.contains("totalNamedConcepts"))
    XCTAssertThrowsError(try SpaceTools.validate(tool: "tonk_space_schema", arguments: .object([:])))
    for key in ["space", "branch", "query", "document"] {
      XCTAssertThrowsError(
        try SpaceTools.validate(
          tool: "tonk_space_schema", arguments: .object([key: .string("override")])))
    }
  }
}
