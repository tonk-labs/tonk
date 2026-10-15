import SwiftUI
import Textual

struct MessageMarkdown: View {
  let text: String

  var body: some View {
    StructuredText(text, parser: ChatMarkdownParser())
      .textual.structuredTextStyle(.gitHub)
      .textual.textSelection(.enabled)
      .font(.body)
      .foregroundStyle(.primary)
  }
}
