import SwiftUI

/// Let the native menu own its anchor and accessibility lifecycle.
struct AnchoredMenuButton<Content: View>: View {
  @ViewBuilder var content: () -> Content

  var body: some View {
    // A plain toolbar menu takes the toolbar's own glass, like its other items.
    Menu(content: content) {
      Label("More Options", systemImage: "ellipsis")
    }
    .menuIndicator(.hidden)
    .help("Refresh, open in browser and sync status")
  }
}
