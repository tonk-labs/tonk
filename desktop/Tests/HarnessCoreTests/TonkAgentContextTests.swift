import XCTest

@testable import HarnessCore

final class TonkAgentContextTests: XCTestCase {
  func testGuideToolAndResourceReturnTheSameCanonicalContent() throws {
    let overview = try TonkAgentContext.guide(.object([:]))
    XCTAssertEqual(overview["usage"].string, TonkAgentContext.instructions)
    for topic in TonkAgentContext.topics {
      let guide = try TonkAgentContext.guide(.object(["topic": .string(topic)]))
      let resource = try TonkAgentContext.resource("tonk://guide/" + topic)
      XCTAssertEqual(guide["text"], resource["contents"].array.first?["text"])
      XCTAssertFalse(guide["text"].string?.isEmpty ?? true)
    }
    for args: JSONValue in [
      .object(["topic": .string("../../private")]),
      .object(["topic": .null]), .object(["space": .string("other")]),
    ] {
      XCTAssertThrowsError(try TonkAgentContext.guide(args))
    }
    XCTAssertThrowsError(try TonkAgentContext.resource("file:///private"))
  }
}
