import HarnessCore
import SwiftUI

enum DesktopPage: Hashable {
  case allLinks
  case addressBook
  case account
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
  @State private var creating = false
  @State private var renaming: TonkSpace?
  @State private var removing: TonkSpace?
  /// Spaces whose removal was confirmed and is still settling.
  @State private var leaving: Set<String> = []
  @Environment(\.appearsActive) private var appearsActive

  private var spaces: [TonkSpace] {
    runtime.spaces.filter { search.isEmpty || $0.title.localizedCaseInsensitiveContains(search) }
  }

  var body: some View {
    List(selection: $selection) {
      Section {
        Label {
          Text("All links")
        } icon: {
          DesktopGlyph(name: "imgIconLink")
        }
        .tag(DesktopPage.allLinks)
        Label {
          Text("Address book")
        } icon: {
          DesktopGlyph(name: "imgIconBookUser")
        }
        .badge(
          AddressBookEntry.entries(
            contacts: access.contacts, grants: access.connections
          ).count
        )
        .tag(DesktopPage.addressBook)
      }
      Section {
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
          .opacity(leaving.contains(space.id) ? 0.5 : 1)
          .contextMenu {
            Button("Rename…") { renaming = space }
              .disabled(!canManage || leaving.contains(space.id))
            Button("Change Icon…") { customizing = space.id }
              .disabled(!runtime.accountConnected)
            Divider()
            Button(removal(space).verb + "…", role: .destructive) { removing = space }
              .disabled(!canManage || leaving.contains(space.id))
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
          Text("No matching spaces").foregroundStyle(.secondary)
        }
        Button {
          creating = true
        } label: {
          Label("New Space…", systemImage: "plus")
        }
        .buttonStyle(.plain).foregroundStyle(.secondary).disabled(!canManage)
      } header: {
        Text("Spaces")
      }
    }
    .listStyle(.sidebar)
    .font(.system(size: 13))
    .environment(\.defaultMinListRowHeight, 28)
    .controlSize(.small)
    .searchable(text: $search, placement: .sidebar, prompt: "Search spaces")
    .searchFocused(searchFocused)
    .sidebarFooter(footer)
    .sheet(isPresented: $creating) {
      SpaceNameSheet(title: "New Space", confirm: "Create") { name in
        let space = try await runtime.createSpace(named: name)
        selection = .space(space.id)
      }
    }
    .sheet(item: $renaming) { space in
      SpaceNameSheet(title: "Rename Space", confirm: "Rename", name: space.title) { name in
        try await runtime.renameSpace(space, to: name)
      }
    }
    .confirmationDialog(
      removing.map { removal($0).title($0.title) } ?? "",
      isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
      presenting: removing
    ) { space in
      Button(removal(space).verb + " Space", role: .destructive) { remove(space) }
    } message: { space in
      Text(removal(space).message)
    }
  }

  /// Creating, renaming and removing need the live catalog, not the cached one.
  private var canManage: Bool {
    runtime.accountConnected && runtime.catalogLoaded && !runtime.showingCachedCatalog
  }

  private func removal(_ space: TonkSpace) -> SpaceRemoval {
    space.removal(accountRoot: runtime.liveAccountRoot)
  }

  private func remove(_ space: TonkSpace) {
    leaving.insert(space.id)
    Task {
      defer { leaving.remove(space.id) }
      do {
        try await runtime.removeSpace(space)
      } catch {
        access.notice =
          "Couldn’t \(removal(space).verb.lowercased()) “\(space.title)”: "
          + AccessModel.message(error)
      }
    }
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

  /// Matches the list's own selection: accent while the window is key, grey otherwise.
  private var footerSelection: Color {
    guard selection == .account else { return .clear }
    return appearsActive
      ? .accentColor : Color(nsColor: .unemphasizedSelectedContentBackgroundColor)
  }

  private var footer: some View {
    VStack(alignment: .leading, spacing: 0) {
      HStack(spacing: 8) {
        Button(action: onSettings) {
          HStack(spacing: 8) {
            Text(AccessText.initials(access.accountName ?? ""))
              .font(.caption.weight(.semibold))
              .frame(width: 22, height: 22)
              .background(.fill.secondary, in: Circle())
              .accessibilityHidden(true)
            Text(
              access.accountName ?? (runtime.accountConnected ? "Your Account" : "Not Signed In")
            )
            .lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
          }.contentShape(Rectangle())
        }.buttonStyle(.plain).help("Settings")
        Button(action: onSettings) {
          Label("Settings", systemImage: "gearshape").labelStyle(.iconOnly)
        }.buttonStyle(.plain).help("Settings")

      }
      .padding(.horizontal, 8).padding(.vertical, 8)
      .foregroundStyle(selection == .account && appearsActive ? Color.white : Color.primary)
      .background(footerSelection, in: RoundedRectangle(cornerRadius: 8))
      .accessibilityAddTraits(selection == .account ? .isSelected : [])
    }
    .font(.system(size: 13))
    .padding(8)
  }
}

extension View {
  /// The account footer: spaces blur as they scroll under it, as the system's
  /// own bars do, and it stays clear when nothing is beneath.
  @ViewBuilder
  fileprivate func sidebarFooter<Footer: View>(_ footer: Footer) -> some View {
    if #available(macOS 26.0, *) {
      safeAreaBar(edge: .bottom, spacing: 0) { footer }
        .scrollEdgeEffectStyle(.soft, for: .bottom)
    } else {
      safeAreaInset(edge: .bottom, spacing: 0) {
        footer.background(.bar).overlay(alignment: .top) { Divider() }
      }
    }
  }
}
