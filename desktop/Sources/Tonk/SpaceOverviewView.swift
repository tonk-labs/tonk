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
    Form {
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
      Section {
        LabeledContent("People") {
          if let state, !state.members.isEmpty {
            Roster(summary: state.peopleSummary, members: state.members)
          } else {
            Text(state == nil ? "Loading…" : "Only you so far")
          }
        }
      }
      Section("Views") {
        let views =
          state?.views ?? [SpaceViewEntry(model: "tonk/space", title: "Home", isHome: true)]
        ForEach(views) { view in
          Button {
            onOpenView(view)
          } label: {
            HStack(spacing: 12) {
              Image(systemName: view.isHome ? "house" : "macwindow")
                .foregroundStyle(.secondary).frame(width: 20)
              VStack(alignment: .leading, spacing: 2) {
                Text(view.title).foregroundStyle(.primary)
                Text(view.isHome ? "Main view" : "List view")
                  .font(.callout).foregroundStyle(.secondary)
              }
              Spacer()
              Image(systemName: "arrow.up.forward").foregroundStyle(.secondary)
            }
            .contentShape(Rectangle())
          }
          .buttonStyle(.plain)
          .accessibilityLabel("Open \(view.title)")
          .help("Open \(view.title)")
        }
      }
      links
    }
    .formStyle(.grouped)
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
    let rows = state?.links ?? []
    let agents = state?.liveAgents.count ?? 0
    Section {
      if rows.isEmpty {
        HStack {
          Text(state == nil ? "Loading links…" : "No links yet.").foregroundStyle(.secondary)
          Spacer()
          if state != nil { Button("New Link…", action: onNewLink) }
        }
      } else {
        ForEach(rows) { link in
          AccessLinkRow(link: link, space: space, access: access)
        }
      }
    } header: {
      HStack {
        Text("Access")
        Spacer()
        Text(
          "\(rows.count) link\(rows.count == 1 ? "" : "s") · \(agents) agent\(agents == 1 ? "" : "s")"
        )
        .foregroundStyle(.secondary)
      }
    }
  }
}
