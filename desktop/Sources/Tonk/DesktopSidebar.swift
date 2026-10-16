import HarnessCore
import SwiftUI

enum DesktopPage: Hashable {
  case allLinks
  case agentLinks
  case space(String)
}

struct DesktopSidebar: View {
  @ObservedObject var runtime: RuntimeModel
  @ObservedObject var access: AccessModel
  @Binding var selection: DesktopPage?
  @Binding var search: String
  var searchFocused: FocusState<Bool>.Binding
  var onNewLink: () -> Void
  var onSyncStatus: () -> Void
  var onSettings: () -> Void
  var onSignOut: (() -> Void)?

  private var spaces: [TonkSpace] {
    runtime.spaces.filter { search.isEmpty || $0.title.localizedCaseInsensitiveContains(search) }
  }

  var body: some View {
    VStack(spacing: 0) {
      ScrollView {
        VStack(alignment: .leading, spacing: 2) {
          SidebarRow(title: "New link", symbol: "plus", action: onNewLink)
            .disabled(runtime.spaces.isEmpty || !runtime.accountConnected)
          row(.allLinks, title: "All links", symbol: "link", count: access.allLinks.count)
          row(.agentLinks, title: "For agents", symbol: "cpu", count: access.agentLinks.count)
          Text("Spaces").font(.subheadline.weight(.medium)).foregroundStyle(.secondary)
            .padding(.horizontal, 10).padding(.top, 18).padding(.bottom, 4)
          ForEach(spaces) { space in
            let shared = (access.spaces[space.id]?.members.count ?? 0) > 1
            row(
              .space(space.id), title: space.title,
              symbol: shared ? "person.2" : "square.grid.2x2",
              count: access.links(for: space.id).count)
          }
          if spaces.isEmpty && !search.isEmpty {
            Text("No matching spaces").foregroundStyle(.secondary).padding(10)
          }
        }
        .padding(.horizontal, 10).padding(.top, 8)
      }
      footer
    }
    .searchable(text: $search, placement: .sidebar, prompt: "Spaces")
    .searchFocused(searchFocused)
  }

  private func row(_ page: DesktopPage, title: String, symbol: String, count: Int) -> some View {
    SidebarRow(
      title: title, symbol: symbol, count: count, selected: selection == page,
      action: { selection = page })
  }

  private var syncLabel: String {
    SyncSummary.label(runtime.spaces.compactMap { access.syncStates[$0.id] })
  }

  private var footer: some View {
    VStack(alignment: .leading, spacing: 12) {
      Divider().padding(.horizontal, -16)
      Button(action: onSyncStatus) {
        HStack(spacing: 8) {
          Circle().fill(syncLabel == "Synced" ? Color.tonkInk : Color.secondary)
            .frame(width: 8, height: 8)
          Text(syncLabel).foregroundStyle(.secondary)
        }
      }
      .buttonStyle(.plain)
      .help("Sync status")
      HStack(spacing: 10) {
        let name =
          access.accountName ?? (runtime.accountConnected ? "Your account" : "Not signed in")
        Text(AccessText.initials(access.accountName ?? ""))
          .font(.caption.weight(.semibold))
          .frame(width: 28, height: 28)
          .background(.quaternary, in: Circle())
          .accessibilityHidden(true)
        Text(name).lineLimit(1)
        Spacer(minLength: 4)
        Menu {
          Button("Settings…", action: onSettings)
          if let onSignOut {
            Divider()
            Button("Sign out of Tonk", action: onSignOut)
          }
        } label: {
          Image(systemName: "gearshape")
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .accessibilityLabel("Account and settings")
      }
    }
    .padding(.horizontal, 16).padding(.bottom, 14)
  }
}

/// One sidebar destination: icon, title, quiet count, tinted when selected.
private struct SidebarRow: View {
  let title: String
  let symbol: String
  var count: Int?
  var selected = false
  let action: () -> Void
  @Environment(\.isEnabled) private var enabled
  @State private var hovered = false

  var body: some View {
    Button(action: action) {
      HStack(spacing: 10) {
        Image(systemName: symbol).frame(width: 20).foregroundStyle(.secondary)
        Text(title).lineLimit(1)
          .fontWeight(selected ? .semibold : .regular)
          .foregroundStyle(selected ? Color.tonkInk : .primary)
        Spacer(minLength: 6)
        if let count, count > 0 {
          Text("\(count)").monospacedDigit().foregroundStyle(.secondary)
        }
      }
      .padding(.horizontal, 10).padding(.vertical, 7)
      .background(
        selected
          ? Color.tonkInk.opacity(0.12) : (hovered ? Color.primary.opacity(0.05) : .clear),
        in: RoundedRectangle(cornerRadius: 8)
      )
      .contentShape(RoundedRectangle(cornerRadius: 8))
    }
    .buttonStyle(.plain)
    .opacity(enabled ? 1 : 0.5)
    .onHover { hovered = $0 }
    .help(title)
    .accessibilityAddTraits(selected ? .isSelected : [])
    .accessibilityValue(count.map { "\($0) links" } ?? "")
  }
}
