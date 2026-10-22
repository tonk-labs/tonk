import HarnessCore
import SwiftUI

struct LinksPageView: View {
  let agentsOnly: Bool
  let spaces: [TonkSpace]
  @ObservedObject var access: AccessModel
  var onNewLink: () -> Void

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 24) {
        ForEach(spaces) { space in
          let rows = access.links(for: space.id).filter { !$0.forAgents }
          if let error = access.errors[space.id] {
            VStack(alignment: .leading, spacing: 8) {
              Text(space.title).fontWeight(.semibold)
              VStack(alignment: .leading, spacing: 10) {
                Text(error).foregroundStyle(.secondary)
                Divider()
                Button("Try Again") { Task { await access.refresh(space) } }
              }.padding(14).frame(maxWidth: .infinity, alignment: .leading)
                .background(DesktopStyle.group, in: RoundedRectangle(cornerRadius: 10))
            }
          }
          if !rows.isEmpty {
            VStack(alignment: .leading, spacing: 8) {
              HStack {
                Text(space.title).fontWeight(.semibold)
                Spacer()
                Text("\(rows.count) link\(rows.count == 1 ? "" : "s")").foregroundStyle(.tertiary)
              }
              VStack(spacing: 0) {
                ForEach(rows) { link in
                  if link.id != rows.first?.id { Divider() }
                  AccessLinkRow(link: link, space: space, access: access)
                    .padding(.horizontal, 14).padding(.vertical, 10)
                }
              }.background(DesktopStyle.group, in: RoundedRectangle(cornerRadius: 10))
            }
          }
        }
        if access.allLinks.filter({ !$0.forAgents }).isEmpty {
          HStack {
            Text(access.loadedAll ? "No links yet." : "Loading links…").foregroundStyle(.secondary)
            Spacer()
            Button("Share a Space…", action: onNewLink).disabled(spaces.isEmpty)
          }
        }
      }.padding(20).frame(maxWidth: .infinity, alignment: .leading)
    }.font(.system(size: 13))
  }
}

/// Keyboard/menu entry point when no space is selected.
struct NewLinkSheet: View {
  let spaces: [TonkSpace]
  @ObservedObject var access: AccessModel
  @State var subject: String
  @Environment(\.dismiss) private var dismiss
  var body: some View {
    VStack(alignment: .leading, spacing: 20) {
      Text("Share a Space").font(.system(size: 15, weight: .semibold))
      Picker("Space", selection: $subject) {
        ForEach(spaces) { Text($0.title).tag($0.id) }
      }
      HStack {
        Button("Done") { dismiss() }.keyboardShortcut(.cancelAction)
        Spacer()
        if let space = spaces.first(where: { $0.id == subject }) {
          SpaceShareControl(space: space, access: access)
        }
      }
    }.padding(22).frame(width: 420)
  }
}
