import HarnessCore
import SwiftUI

/// A space at a glance: who is in it, its views, what changed, and its links.
struct SpaceOverviewView: View {
  let space: TonkSpace
  @ObservedObject var access: AccessModel
  var onOpenView: (SpaceViewEntry) -> Void
  var onNewLink: () -> Void

  private var state: SpaceAccess? { access.spaces[space.id] }

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 28) {
        if let error = access.errors[space.id] {
          HStack(alignment: .top) {
            Image(systemName: "exclamationmark.circle")
            Text("Couldn’t load everything for this space: \(error)")
              .textSelection(.enabled)
            Spacer()
            Button("Try again") { Task { await access.refresh(space) } }
          }
          .padding(14)
          .background(.quaternary, in: RoundedRectangle(cornerRadius: 12))
        }
        people
        views
        activity
        links
      }
      .frame(maxWidth: 820, alignment: .leading)
      .padding(.horizontal, 40).padding(.vertical, 32)
      .frame(maxWidth: .infinity)
    }
    .overlay {
      if state == nil && access.errors[space.id] == nil {
        ProgressView().controlSize(.large)
      }
    }
    .task(id: space.id) { await access.refresh(space) }
  }

  private var people: some View {
    OverviewCard {
      HStack(spacing: 16) {
        Text("People").foregroundStyle(.secondary).frame(width: 120, alignment: .leading)
        if let state, !state.members.isEmpty {
          InitialsStack(members: state.members)
          Text(state.peopleSummary).lineLimit(1)
        } else {
          Text(state == nil ? "Loading…" : "Only you so far").foregroundStyle(.secondary)
        }
        Spacer(minLength: 0)
      }
      .padding(.horizontal, 20).padding(.vertical, 16)
    }
  }

  @ViewBuilder private var views: some View {
    VStack(alignment: .leading, spacing: 12) {
      SectionHeading(title: "Views")
      LazyVGrid(columns: [GridItem(.adaptive(minimum: 180), spacing: 14)], spacing: 14) {
        ForEach(state?.views ?? [SpaceViewEntry(model: "tonk/space", title: "Home", isHome: true)])
        { view in
          Button {
            onOpenView(view)
          } label: {
            VStack(alignment: .leading, spacing: 0) {
              HStack(alignment: .top) {
                Image(systemName: "macwindow").font(.title3)
                  .frame(width: 38, height: 38)
                  .background(.quaternary, in: RoundedRectangle(cornerRadius: 9))
                Spacer()
                Image(systemName: "arrow.up.right").foregroundStyle(.secondary)
              }
              Spacer(minLength: 18)
              Text(view.title).font(.headline).lineLimit(1)
              Text(view.isHome ? "Main view" : "Directory").foregroundStyle(.secondary)
            }
            .padding(18)
            .frame(maxWidth: .infinity, minHeight: 130, alignment: .leading)
            .background(
              Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 14)
            )
            .overlay {
              RoundedRectangle(cornerRadius: 14)
                .strokeBorder(Color(nsColor: .separatorColor), lineWidth: 1)
            }
            .contentShape(RoundedRectangle(cornerRadius: 14))
          }
          .buttonStyle(.plain)
          .accessibilityLabel("Open \(view.title)")
        }
      }
    }
  }

  private var activity: some View {
    VStack(alignment: .leading, spacing: 12) {
      SectionHeading(title: "Recent activity", trailing: "On this Mac")
      OverviewCard {
        let events = access.journal.recent(subject: space.subject)
        if events.isEmpty {
          Text("Links you make, reset or revoke here will appear here.")
            .foregroundStyle(.secondary).padding(20)
        } else {
          TimelineView(.periodic(from: .now, by: 60)) { context in
            VStack(alignment: .leading, spacing: 10) {
              ForEach(events) { event in
                HStack(alignment: .firstTextBaseline, spacing: 16) {
                  Text(AccessText.relative(event.date, now: context.date))
                    .foregroundStyle(.secondary).frame(width: 110, alignment: .leading)
                  Text(event.sentence)
                }
              }
            }
            .padding(.horizontal, 20).padding(.vertical, 16)
          }
        }
      }
    }
  }

  @ViewBuilder private var links: some View {
    let rows = state?.links ?? []
    let agents = state?.liveAgents.count ?? 0
    VStack(alignment: .leading, spacing: 12) {
      SectionHeading(
        title: "Access",
        trailing:
          "\(rows.count) link\(rows.count == 1 ? "" : "s") · \(agents) agent\(agents == 1 ? "" : "s")"
      )
      OverviewCard {
        if rows.isEmpty {
          HStack {
            Text(
              state == nil ? "Loading links…" : "No links yet. Make one to invite people or agents."
            )
            .foregroundStyle(.secondary)
            Spacer()
            if state != nil { Button("New link", action: onNewLink) }
          }
          .padding(20)
        } else {
          ForEach(Array(rows.enumerated()), id: \.element.id) { index, link in
            if index > 0 { Divider() }
            AccessLinkRow(link: link, space: space, access: access)
          }
        }
      }
    }
  }
}
