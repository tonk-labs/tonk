import HarnessCore
import SwiftUI

/// A space at a glance: who is in it, its views, what changed, and its links.
struct SpaceOverviewView: View {
  let space: TonkSpace
  @ObservedObject var access: AccessModel
  var canRefresh = true
  var onOpenView: (SpaceViewEntry) -> Void
  var onNewLink: () -> Void

  private var state: SpaceAccess? { access.spaces[space.id] }

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 24) {
        if let error = access.errors["account"] {
          Section {
            Text("Agent connections could not be refreshed: " + error)
            Button("Try Again") {
              Task {
                await access.refreshAccount()
                await access.refresh(space)
              }
            }
          }
        }
        if let error = access.errors[space.id] {
          Section {
            Label {
              Text("Couldn’t load everything for this space: \(error)").textSelection(.enabled)
            } icon: {
              Image(systemName: "exclamationmark.triangle").foregroundStyle(.orange)
            }
            Button("Try Again") { Task { await access.refresh(space) } }
          }
        }
        Group {
          HStack(spacing: 12) {
            Text("People").foregroundStyle(.secondary).frame(width: 124, alignment: .leading)
            if let state, !state.members.isEmpty {
              Roster(summary: state.peopleSummary, members: state.members)
            } else {
              Text(state == nil ? "Loading…" : "Only you so far")
            }
          }
        }.padding(.horizontal, 14).padding(.vertical, 10).frame(
          maxWidth: .infinity, alignment: .leading
        )
        .background(DesktopStyle.group, in: RoundedRectangle(cornerRadius: 10))
        VStack(alignment: .leading, spacing: 8) {
          Text("Views").font(.system(size: 13, weight: .semibold))
          LazyVGrid(columns: [GridItem(.adaptive(minimum: 150), spacing: 8)], spacing: 8) {
            let views =
              state?.views ?? [SpaceViewEntry(model: "tonk/space", title: "Home", isHome: true)]
            ForEach(views) { view in
              Button {
                onOpenView(view)
              } label: {
                VStack(alignment: .leading, spacing: 2) {
                  HStack(alignment: .top) {
                    DesktopGlyph(name: "imgIconAppWindow")
                      .frame(width: 28, height: 28)
                      .background(.quaternary, in: RoundedRectangle(cornerRadius: 7))
                    Spacer()
                    DesktopGlyph(name: "imgIconArrowUpRight", size: 14).foregroundStyle(.tertiary)
                  }.padding(.bottom, 8)
                  Text(view.title).fontWeight(.medium).foregroundStyle(.primary).lineLimit(1)
                  Text(view.isHome ? "Main view" : "View")
                    .font(.system(size: 12)).foregroundStyle(.secondary).lineLimit(1)
                }
                .font(.system(size: 13))
                .padding(12).frame(maxWidth: .infinity, alignment: .leading)
                .background(DesktopStyle.group, in: RoundedRectangle(cornerRadius: 10))
                .contentShape(RoundedRectangle(cornerRadius: 10))
              }
              .buttonStyle(.plain)
              .accessibilityLabel("Open \(view.title)")
              .help("Open \(view.title)")
            }
          }
        }
        links
      }.padding(20).frame(maxWidth: .infinity, alignment: .leading)
    }
    .font(.system(size: 13))
    .overlay {
      if state == nil && access.errors[space.id] == nil {
        ProgressView()
      }
    }
    .task(id: canRefresh ? space.id : nil) {
      guard canRefresh else { return }
      await access.refresh(space)
    }
  }

  @ViewBuilder private var links: some View {
    let rows = (state?.links ?? []).filter { !$0.forAgents }
    VStack(alignment: .leading, spacing: 8) {
      Text("Links").font(.system(size: 13, weight: .semibold))
      VStack(spacing: 0) {
        if rows.isEmpty {
          HStack {
            Text(state == nil ? "Loading links…" : "No links yet.").foregroundStyle(.secondary)
            Spacer()
            if state != nil { Button("Copy link", action: onNewLink) }
          }.padding(14)
        } else {
          ForEach(rows) { link in
            if link.id != rows.first?.id { Divider() }
            AccessLinkRow(link: link, space: space, access: access).padding(.horizontal, 14)
              .padding(.vertical, 10)
          }
        }
      }.background(DesktopStyle.group, in: RoundedRectangle(cornerRadius: 10))
    }
  }
}
