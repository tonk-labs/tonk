import SwiftUI

/// Glass belongs to the control layer; transcripts and hosted space content stay opaque.
struct GlassSurface: ViewModifier {
  @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
  var radius: CGFloat

  @ViewBuilder
  func body(content: Content) -> some View {
    if reduceTransparency {
      content.background(
        Color(nsColor: .windowBackgroundColor), in: RoundedRectangle(cornerRadius: radius))
    } else if #available(macOS 26.0, *) {
      content.glassEffect(.regular, in: RoundedRectangle(cornerRadius: radius))
    } else {
      content.background(.regularMaterial, in: RoundedRectangle(cornerRadius: radius))
    }
  }
}

extension View {
  func controlSurface(radius: CGFloat = 20) -> some View {
    modifier(GlassSurface(radius: radius))
  }
}

struct NativeControlStyle: ViewModifier {
  var prominent = false
  var circular = false
  private let primaryTint = Color(red: 0.16, green: 0.46, blue: 0.86)

  @ViewBuilder
  func body(content: Content) -> some View {
    if #available(macOS 26.0, *) {
      if prominent {
        content.buttonStyle(.glassProminent).tint(primaryTint).buttonBorderShape(
          circular ? .circle : .capsule)
      } else {
        content.buttonStyle(.glass).buttonBorderShape(circular ? .circle : .capsule)
      }
    } else if prominent {
      content.buttonStyle(.borderedProminent).tint(primaryTint)
    } else {
      content.buttonStyle(.bordered)
    }
  }
}

extension View {
  func nativeControl(prominent: Bool = false, circular: Bool = false) -> some View {
    modifier(NativeControlStyle(prominent: prominent, circular: circular))
  }
}

/// Related glass controls share a sampling container, without stacking glass surfaces.
struct GlassControls<Content: View>: View {
  @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
  @ViewBuilder var content: () -> Content

  @ViewBuilder var body: some View {
    if #available(macOS 26.0, *), !reduceTransparency {
      GlassEffectContainer(spacing: 8) { content() }
    } else {
      content()
    }
  }
}

extension ToolbarContent {
  /// Custom glass controls already supply their own material.
  @ToolbarContentBuilder
  func customGlassToolbarItem() -> some ToolbarContent {
    if #available(macOS 26.0, *) {
      sharedBackgroundVisibility(.hidden)
    } else {
      self
    }
  }
}
