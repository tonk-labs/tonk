import SwiftUI

/// Shared grouped surfaces from the Page 0 desktop wireframes.
enum DesktopStyle {
  static let group = Color("DesktopGroup", bundle: .module)
  static let accent = Color("DesktopAccent", bundle: .module)
}

/// Vector assets exported from the Page 0 Figma file, compiled by the asset catalog.
struct DesktopGlyph: View {
  let name: String
  var size: CGFloat = 16
  var body: some View {
    Image(name, bundle: .module).renderingMode(.template)
      .resizable().scaledToFit().frame(width: size, height: size)
      .accessibilityHidden(true)
  }
}
