import XCTest

@testable import Tonk

@MainActor
final class ChatMarkdownParserTests: XCTestCase {
  func testTaskMarkersPreserveCodeAndOtherMarkdown() throws {
    let rendered = try ChatMarkdownParser().attributedString(
      for: """
        - [x] **Done**
        - [ ] Pending
          - [X] Nested
        - `[x] literal`

        ```text
        - [x] code
        ```

        [x] plain paragraph

        | Label | State |
        | --- | --- |
        | A | true |
        """)
    let text = String(rendered.characters)
    XCTAssertTrue(text.contains("\u{2611}\u{FE0E} Done"))
    XCTAssertTrue(text.contains("\u{2610}\u{FE0E} Pending"))
    XCTAssertTrue(text.contains("\u{2611}\u{FE0E} Nested"))
    XCTAssertTrue(text.contains("[x] literal"))
    XCTAssertTrue(text.contains("- [x] code"))
    XCTAssertTrue(text.contains("[x] plain paragraph"))
    XCTAssertTrue(
      rendered.runs.contains { $0.inlinePresentationIntent?.contains(.stronglyEmphasized) == true })
    XCTAssertTrue(
      rendered.runs.contains {
        $0.presentationIntent?.components.contains {
          if case .table = $0.kind { return true }
          return false
        } == true
      })
  }
}
