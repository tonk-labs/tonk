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
    List(selection: $selection) {
      Section {
        Label("All Links", systemImage: "link")
          .badge(access.allLinks.count)
          .tag(DesktopPage.allLinks)
        Label("For Agents", systemImage: "cpu")
          .badge(access.agentLinks.count)
          .tag(DesktopPage.agentLinks)
      }
      Section("Spaces") {
        ForEach(spaces) { space in
          let shared = (access.spaces[space.id]?.members.count ?? 0) > 1
          Label(space.title, systemImage: shared ? "person.2" : "square.grid.2x2")
            .badge(access.links(for: space.id).count)
            .tag(DesktopPage.space(space.id))
            .help(space.title)
        }
        if spaces.isEmpty && !search.isEmpty {
          Text("No Matching Spaces").foregroundStyle(.secondary)
        }
      }
    }
    .listStyle(.sidebar)
    .searchable(text: $search, placement: .sidebar, prompt: "Search Spaces")
    .searchFocused(searchFocused)
    .safeAreaInset(edge: .bottom, spacing: 0) { footer }
  }

  private var sync: (label: String, symbol: String, color: Color) {
    let label = SyncSummary.label(runtime.spaces.compactMap { access.syncStates[$0.id] })
    return switch label {
    case "Synced": (label, "checkmark.circle.fill", .green)
    case "Syncing", "Checking sync": (label, "arrow.triangle.2.circlepath", .secondary)
    case "On this Mac": (label, "laptopcomputer", .secondary)
    default: (label, "exclamationmark.triangle.fill", .orange)
    }
  }

  private var footer: some View {
    VStack(alignment: .leading, spacing: 10) {
      Button(action: onSyncStatus) {
        Label {
          Text(sync.label).foregroundStyle(.secondary)
        } icon: {
          Image(systemName: sync.symbol).foregroundStyle(sync.color)
        }
      }
      .buttonStyle(.plain)
      .help("Show sync status")
      HStack(spacing: 8) {
        Text(AccessText.initials(access.accountName ?? ""))
          .font(.caption.weight(.semibold))
          .frame(width: 26, height: 26)
          .background(.fill.secondary, in: Circle())
          .accessibilityHidden(true)
        Text(access.accountName ?? (runtime.accountConnected ? "Your Account" : "Not Signed In"))
          .lineLimit(1)
        Spacer(minLength: 4)
        Menu {
          Button("New Link…", action: onNewLink)
            .disabled(runtime.spaces.isEmpty || !runtime.accountConnected)
          Divider()
          Button("Settings…", action: onSettings)
          if let onSignOut {
            Divider()
            Button("Sign Out of Tonk", action: onSignOut)
          }
        } label: {
          Image(systemName: "gearshape")
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("Account and settings")
        .accessibilityLabel("Account and settings")
      }
    }
    .padding(.horizontal, 16).padding(.vertical, 12)
    .overlay(alignment: .top) { Divider() }
  }
}
