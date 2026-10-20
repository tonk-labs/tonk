import HarnessCore
import SwiftUI

enum DesktopPage: Hashable {
  case allLinks
  case addressBook
  case account
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
  var onSettings: () -> Void
  var onSignOut: (() -> Void)?
  /// The space whose icon picker is open, from its context menu.
  @State private var customizing: String?
  @State private var hovered: String?

  private var spaces: [TonkSpace] {
    runtime.spaces.filter { search.isEmpty || $0.title.localizedCaseInsensitiveContains(search) }
  }

  var body: some View {
    List(selection: $selection) {
      Section {
        Label {
          Text("All Links")
        } icon: {
          DesktopGlyph(name: "imgIconLink")
        }
        .tag(DesktopPage.allLinks)
        Label {
          Text("Connections")
        } icon: {
          DesktopGlyph(name: "imgIconPlug")
        }
        .badge(access.agentLinks.count)
        .tag(DesktopPage.agentLinks)
        Label {
          Text("Address Book")
        } icon: {
          DesktopGlyph(name: "imgIconBookUser")
        }
        .badge(access.contacts.count)
        .tag(DesktopPage.addressBook)
      }
      Section("Spaces") {
        ForEach(spaces) { space in
          let shared = (access.spaces[space.id]?.members.count ?? 0) > 1
          Label {
            Text(space.title)
          } icon: {
            EditableSpaceIcon(
              space: space, shared: shared, hovering: hovered == space.id,
              enabled: runtime.accountConnected
            ) { customizing = space.id }
          }
          .tag(DesktopPage.space(space.id))
          .help(space.title)
          .onHover { over in
            if over {
              hovered = space.id
            } else if hovered == space.id {
              hovered = nil
            }
          }
          .contextMenu {
            Button("Change Icon…") { customizing = space.id }
              .disabled(!runtime.accountConnected)
          }
          .popover(
            isPresented: Binding(
              get: { customizing == space.id },
              set: { if !$0 { customizing = nil } }),
            arrowEdge: .trailing
          ) {
            SpaceAppearancePicker(space: space, shared: shared) { icon, color in
              save(space, icon: icon, color: color)
            }
          }
        }
        if spaces.isEmpty && !search.isEmpty {
          Text("No Matching Spaces").foregroundStyle(.secondary)
        }
      }
    }
    .listStyle(.sidebar)
    .font(.system(size: 13))
    .environment(\.defaultMinListRowHeight, 28)
    .controlSize(.small)
    .searchable(text: $search, placement: .sidebar, prompt: "Search Spaces")
    .searchFocused(searchFocused)
    .safeAreaInset(edge: .bottom, spacing: 0) { footer }
  }

  private func save(_ space: TonkSpace, icon: String?, color: String?) {
    Task {
      do {
        try await runtime.setSpaceAppearance(space, icon: icon, color: color)
      } catch {
        access.notice = "Couldn’t save the icon: " + error.localizedDescription
      }
    }
  }

  private var footer: some View {
    VStack(alignment: .leading, spacing: 0) {
      HStack(spacing: 8) {
        Text(AccessText.initials(access.accountName ?? ""))
          .font(.caption.weight(.semibold))
          .frame(width: 22, height: 22)
          .background(.fill.secondary, in: Circle())
          .accessibilityHidden(true)
        Button {
          selection = .account
        } label: {
          Text(access.accountName ?? (runtime.accountConnected ? "Your Account" : "Not Signed In"))
            .lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
        }.buttonStyle(.plain).help("Account")
        Spacer(minLength: 4)
        Menu {
          Button("New Link…", action: onNewLink)
            .disabled(runtime.spaces.isEmpty || !runtime.accountConnected)
          Divider()
          Button("Account") { selection = .account }
          Button("Chat Agent Settings…", action: onSettings)
          if let onSignOut {
            Divider()
            Button("Sign Out of Tonk", action: onSignOut)
          }
        } label: {
          // A titled label keeps its name; a bare symbol reads as "Gear Shape".
          Label("Account and Settings", systemImage: "gearshape").labelStyle(.iconOnly)
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("Account and Settings")
      }
    }
    .font(.system(size: 13))
    .padding(.horizontal, 10).padding(.vertical, 10)
    .overlay(alignment: .top) { Divider() }
  }
}
