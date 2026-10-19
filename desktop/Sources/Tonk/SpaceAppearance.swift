import HarnessCore
import SwiftUI

/// The icons and tints a space can wear in the sidebar. They are stored as
/// plain tokens on the account directory, so only these short lists are
/// honoured and anything else falls back to the default.
enum SpaceAppearance {
  static let icons = [
    "square.grid.2x2", "person.2", "book", "checklist", "folder", "doc.text",
    "star", "heart", "leaf", "bolt", "flag", "calendar",
    "house", "briefcase", "graduationcap", "paintpalette", "music.note", "gamecontroller",
  ]

  static let colors: [(token: String, name: String, color: Color)] = [
    ("red", "Red", .red), ("orange", "Orange", .orange), ("yellow", "Yellow", .yellow),
    ("green", "Green", .green), ("teal", "Teal", .teal), ("blue", "Blue", .blue),
    ("indigo", "Indigo", .indigo), ("purple", "Purple", .purple), ("pink", "Pink", .pink),
    ("brown", "Brown", .brown), ("gray", "Gray", .gray),
  ]

  static func symbol(for space: TonkSpace, shared: Bool) -> String {
    if let icon = space.icon, icons.contains(icon) { return icon }
    return shared ? "person.2" : "square.grid.2x2"
  }

  static func color(_ token: String?) -> Color? {
    colors.first(where: { $0.token == token })?.color
  }
}

/// A space's sidebar icon, tinted when the account chose a color.
struct SpaceIcon: View {
  let space: TonkSpace
  let shared: Bool

  var body: some View {
    let image = Image(systemName: SpaceAppearance.symbol(for: space, shared: shared))
    if let tint = SpaceAppearance.color(space.color) {
      image.foregroundStyle(tint)
    } else {
      image
    }
  }
}

/// A space's sidebar icon. While its row is hovered it turns into an edit
/// button that opens the icon picker, so customising needs no right-click.
struct EditableSpaceIcon: View {
  let space: TonkSpace
  let shared: Bool
  let hovering: Bool
  let enabled: Bool
  var onEdit: () -> Void

  var body: some View {
    let editing = hovering && enabled
    Button(action: onEdit) {
      if editing {
        // The row's own foreground, not the accent: on the selected row the
        // accent matches the selection and the pencil would vanish.
        Image(systemName: "pencil.circle.fill")
          .transition(.scale(scale: 0.6).combined(with: .opacity))
      } else {
        SpaceIcon(space: space, shared: shared)
          .transition(.scale(scale: 0.6).combined(with: .opacity))
      }
    }
    .buttonStyle(.plain)
    // One slot width for every symbol, so swapping in the pencil (or a wider
    // icon) never moves the space's name.
    .frame(width: 22, alignment: .center)
    .allowsHitTesting(editing)
    .animation(.spring(duration: 0.25, bounce: 0.3), value: editing)
    .help(editing ? "Change Icon" : "")
    .accessibilityLabel("Change Icon for \(space.title)")
  }
}

/// Choose a space's sidebar icon and tint. Each choice saves straight away.
struct SpaceAppearancePicker: View {
  let space: TonkSpace
  let shared: Bool
  var onChange: (_ icon: String?, _ color: String?) -> Void
  @State private var icon: String?
  @State private var color: String?

  init(space: TonkSpace, shared: Bool, onChange: @escaping (String?, String?) -> Void) {
    self.space = space
    self.shared = shared
    self.onChange = onChange
    _icon = State(initialValue: space.icon)
    _color = State(initialValue: space.color)
  }

  private var shownIcon: String {
    if let icon, SpaceAppearance.icons.contains(icon) { return icon }
    return shared ? "person.2" : "square.grid.2x2"
  }

  var body: some View {
    VStack(alignment: .leading, spacing: 12) {
      Text("Icon").font(.headline)
      LazyVGrid(columns: Array(repeating: GridItem(.fixed(32), spacing: 6), count: 6), spacing: 6) {
        ForEach(SpaceAppearance.icons, id: \.self) { symbol in
          Button {
            icon = symbol
            onChange(icon, color)
          } label: {
            Image(systemName: symbol)
              .frame(width: 32, height: 32)
              .foregroundStyle(SpaceAppearance.color(color) ?? .primary)
              .background(
                shownIcon == symbol ? AnyShapeStyle(.selection) : AnyShapeStyle(.clear),
                in: RoundedRectangle(cornerRadius: 7))
              .contentShape(RoundedRectangle(cornerRadius: 7))
          }
          .buttonStyle(.plain)
          .accessibilityLabel(symbol)
          .accessibilityAddTraits(shownIcon == symbol ? .isSelected : [])
        }
      }
      Text("Color").font(.headline)
      LazyVGrid(columns: Array(repeating: GridItem(.fixed(32), spacing: 6), count: 6), spacing: 6) {
        swatch(token: nil, name: "Default", fill: AnyShapeStyle(.secondary))
        ForEach(SpaceAppearance.colors, id: \.token) { entry in
          swatch(token: entry.token, name: entry.name, fill: AnyShapeStyle(entry.color))
        }
      }
      HStack {
        Spacer()
        Button("Reset") {
          icon = nil
          color = nil
          onChange(nil, nil)
        }
        .disabled(icon == nil && color == nil)
      }
    }
    .padding(20)
    .frame(width: 262)
  }

  private func swatch(token: String?, name: String, fill: AnyShapeStyle) -> some View {
    Button {
      color = token
      onChange(icon, color)
    } label: {
      Circle().fill(fill).frame(width: 16, height: 16)
        .overlay {
          if color == token {
            Circle().strokeBorder(.primary, lineWidth: 2).frame(width: 22, height: 22)
          }
        }
        .frame(width: 22, height: 22)
        .contentShape(Circle())
    }
    .buttonStyle(.plain)
    .help(name)
    .accessibilityLabel(name)
    .accessibilityAddTraits(color == token ? .isSelected : [])
  }
}
