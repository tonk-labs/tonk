import HarnessCore
import SwiftUI

struct LinksPageView: View {
  let agentsOnly: Bool
  let spaces: [TonkSpace]
  @ObservedObject var access: AccessModel
  var onNewLink: () -> Void

  private var linkCount: Int { access.allLinks.filter { !$0.forAgents }.count }

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
        if linkCount == 0 {
          if access.loadedAll { emptyState } else { LinksPlaceholder() }
        }
      }.padding(20).frame(maxWidth: .infinity, alignment: .leading)
    }.font(.system(size: 13))
  }

  private var emptyState: some View {
    VStack(spacing: 10) {
      DesktopGlyph(name: "imgIconLink", size: 28).foregroundStyle(.tertiary)
      Text("No links yet").font(.headline)
      Text("Share a space to make a link. Links you make for people show up here.")
        .foregroundStyle(.secondary).multilineTextAlignment(.center).frame(maxWidth: 320)
      Button("Share a Space…", action: onNewLink).disabled(spaces.isEmpty)
        .buttonStyle(.borderedProminent)
    }.frame(maxWidth: .infinity).padding(.top, 60)
  }
}

/// The shape of a links group, so the page does not open on blank space. It
/// pulses gently while links load, and holds still under Reduce Motion.
private struct LinksPlaceholder: View {
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  @State private var dim = false

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      Text("Space name").fontWeight(.semibold)
      VStack(spacing: 0) {
        ForEach(0..<3, id: \.self) { index in
          if index > 0 { Divider() }
          VStack(alignment: .leading, spacing: 3) {
            Text("Link for someone")
            Text("Anyone with the link · Read and write").font(.system(size: 12))
          }.padding(.horizontal, 14).padding(.vertical, 10)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
      }.background(DesktopStyle.group, in: RoundedRectangle(cornerRadius: 10))
    }
    .redacted(reason: .placeholder)
    .opacity(dim ? 0.45 : 1)
    .accessibilityHidden(true)
    .onAppear {
      guard !reduceMotion else { return }
      withAnimation(.easeInOut(duration: 0.9).repeatForever(autoreverses: true)) { dim = true }
    }
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
