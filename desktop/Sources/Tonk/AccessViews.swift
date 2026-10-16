import HarnessCore
import SwiftUI

extension Color {
  /// Tonk's ink: prominent actions and the selected space.
  static let tonkInk = Color(red: 0.20, green: 0.09, blue: 0.16)
}

/// A bordered group on the overview: People, Views, activity and links.
struct OverviewCard<Content: View>: View {
  @ViewBuilder var content: () -> Content

  var body: some View {
    VStack(alignment: .leading, spacing: 0) { content() }
      .frame(maxWidth: .infinity, alignment: .leading)
      .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 14))
      .overlay {
        RoundedRectangle(cornerRadius: 14)
          .strokeBorder(Color(nsColor: .separatorColor), lineWidth: 1)
          .allowsHitTesting(false)
      }
  }
}

struct SectionHeading: View {
  let title: String
  var trailing: String?

  var body: some View {
    HStack(alignment: .firstTextBaseline) {
      Text(title).font(.headline)
      Spacer()
      if let trailing { Text(trailing).foregroundStyle(.secondary) }
    }
    .padding(.horizontal, 4)
  }
}

/// Small rounded label, e.g. "for agents".
struct LinkBadge: View {
  let text: String
  var body: some View {
    Text(text).font(.caption.weight(.medium)).foregroundStyle(.secondary)
      .padding(.horizontal, 7).padding(.vertical, 2)
      .background(.quaternary, in: RoundedRectangle(cornerRadius: 5))
  }
}

/// Overlapping initials for a roster.
struct InitialsStack: View {
  let members: [SpaceMember]
  var limit = 6

  var body: some View {
    HStack(spacing: -6) {
      ForEach(members.prefix(limit)) { member in
        Text(member.initial).font(.caption.weight(.semibold))
          .frame(width: 26, height: 26)
          .background(Color.tonkInk.opacity(0.10), in: Circle())
          .overlay { Circle().strokeBorder(Color(nsColor: .textBackgroundColor), lineWidth: 2) }
          .help(member.displayName)
      }
      if members.count > limit {
        Text("+\(members.count - limit)").font(.caption2.weight(.semibold))
          .foregroundStyle(.secondary).padding(.leading, 10)
      }
    }
    .accessibilityElement(children: .ignore)
    .accessibilityLabel(AccessText.list(members.map(\.displayName)))
  }
}

extension AccessLink {
  var symbol: String {
    switch kind {
    case .agent: "cpu"
    case .publicRead: "globe"
    case .person, .scoped: "link"
    }
  }
}

/// One link: what it allows, who used it, and Reset / Revoke / Copy.
struct AccessLinkRow: View {
  let link: AccessLink
  let space: TonkSpace?
  var showsSpace = false
  @ObservedObject var access: AccessModel
  @State private var confirming = false

  var body: some View {
    TimelineView(.periodic(from: .now, by: 60)) { context in
      HStack(alignment: .center, spacing: 14) {
        Image(systemName: link.symbol).font(.body).foregroundStyle(.secondary)
          .frame(width: 22)
          .accessibilityHidden(true)
        VStack(alignment: .leading, spacing: 3) {
          HStack(spacing: 8) {
            Text(showsSpace ? (space?.title ?? "Space") + " · " + link.title : link.title)
              .lineLimit(1)
            if link.forAgents { LinkBadge(text: "for agents") }
          }
          Text(link.detail(now: context.date)).foregroundStyle(.secondary).lineLimit(1)
          if let usage = link.usage(madeAt: access.madeAt(link), now: context.date) {
            Text(usage).foregroundStyle(.tertiary).lineLimit(1)
          }
        }
        Spacer(minLength: 12)
        actions
      }
      .padding(.horizontal, 20).padding(.vertical, 14)
    }
    .confirmationDialog(confirmTitle, isPresented: $confirming) {
      Button(link.canReset ? "Reset link" : confirmAction, role: .destructive) {
        guard let space else { return }
        Task {
          if link.canReset {
            await access.resetLink(link, space: space)
          } else {
            await access.revoke(link, space: space)
          }
        }
      }
    } message: {
      Text(confirmMessage)
    }
  }

  @ViewBuilder private var actions: some View {
    let busy = access.working.contains(link.id)
    HStack(spacing: 8) {
      if busy { ProgressView().controlSize(.small) }
      Button(link.canReset ? "Reset" : (link.kind == .publicRead ? "Make private" : "Revoke")) {
        confirming = true
      }
      .disabled(busy || space == nil)
      let stored = access.storedLink(link)
      Button("Copy") { access.copyLink(link) }
        .disabled(busy || stored == nil)
        .help(
          stored == nil
            ? "This link was made on another device or before this Mac kept links. Reset or make a new one to copy it."
            : "Copy link")
    }
    .buttonStyle(.bordered)
    .controlSize(.regular)
  }

  private var confirmTitle: String {
    switch link.kind {
    case .person where link.canReset: "Reset this link?"
    case .publicRead: "Make this space private?"
    default: "Revoke this link?"
    }
  }

  private var confirmAction: String {
    link.kind == .publicRead ? "Make private" : "Revoke"
  }

  private var confirmMessage: String {
    switch link.kind {
    case .person:
      "A new link is copied for you. The old link stops working; people who already joined keep their access."
    case .scoped:
      "The link stops working. People who already joined keep their access."
    case .agent:
      "Agents using this link lose access to the space."
    case .publicRead:
      "The public address stops working and readers lose access."
    }
  }
}
