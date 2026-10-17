import SwiftUI

/// Let the native menu own its anchor and accessibility lifecycle.
struct AnchoredMenuButton<Content: View>: View {
  @ViewBuilder var content: () -> Content

  var body: some View {
    Menu(content: content) {
      Image(systemName: "ellipsis").frame(width: 20, height: 20)
    }
    .menuStyle(.borderlessButton)
    .menuIndicator(.hidden)
    .fixedSize()
    .nativeControl(circular: true).controlSize(.large)
    .accessibilityLabel("More Options").help("Refresh, account and model settings")
  }
}
