import Foundation
import Textual

/// Preserve task markers when using Foundation's block parser (which treats them as plain text).
struct ChatMarkdownParser: MarkupParser {
  func attributedString(for input: String) throws -> AttributedString {
    var result = try AttributedStringMarkdownParser.markdown().attributedString(for: input)
    var replacements: [(offset: Int, checked: Bool)] = []
    for (intent, range) in result.runs[\.presentationIntent] {
      guard let intent,
        intent.components.contains(where: {
          if case .listItem = $0.kind { return true }
          return false
        }),
        intent.components.contains(where: {
          if case .paragraph = $0.kind { return true }
          return false
        }),
        result[range].runs.first?.inlinePresentationIntent?.contains(.code) != true
      else { continue }
      let prefix = String(result[range].characters.prefix(4))
      guard ["[x] ", "[X] ", "[ ] "].contains(prefix) else { continue }
      replacements.append(
        (
          result.characters.distance(from: result.startIndex, to: range.lowerBound),
          prefix != "[ ] "
        ))
    }
    for replacement in replacements.reversed() {
      let start = result.characters.index(result.startIndex, offsetBy: replacement.offset)
      let end = result.characters.index(start, offsetBy: 3)
      // Text-presentation ballot boxes remain selectable and copyable with the task label.
      var marker = AttributedString(replacement.checked ? "\u{2611}\u{FE0E}" : "\u{2610}\u{FE0E}")
      marker.setAttributes(result[start..<end].runs.first!.attributes)
      result.replaceSubrange(start..<end, with: marker)
    }
    return result
  }
}
