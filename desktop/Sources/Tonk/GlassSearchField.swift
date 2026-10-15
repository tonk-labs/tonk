import SwiftUI

/// A shared search control with native text editing and a visible keyboard focus ring.
struct GlassSearchField: View {
  let title: String
  @Binding var text: String
  @FocusState private var focused: Bool

  var body: some View {
    HStack(spacing: 8) {
      Image(systemName: "magnifyingglass").foregroundStyle(.secondary).accessibilityHidden(true)
      TextField(title, text: $text)
        .textFieldStyle(.plain).focused($focused).accessibilityLabel(title)
        .onKeyPress(.escape) {
          guard !text.isEmpty else { return .ignored }
          text = ""
          return .handled
        }
      if !text.isEmpty {
        Button("Clear search", systemImage: "xmark.circle.fill") {
          text = ""
          focused = true
        }.labelStyle(.iconOnly).buttonStyle(.plain)
          .frame(width: 24, height: 24)
      }
    }
    .padding(.horizontal, 12).frame(height: 36)
    .controlSurface(radius: 18)
    .overlay {
      Capsule().strokeBorder(focused ? Color.accentColor : .clear, lineWidth: 2)
        .allowsHitTesting(false)
    }
    .focusedSceneValue(\.focusSearch, { focused = true })
  }
}
