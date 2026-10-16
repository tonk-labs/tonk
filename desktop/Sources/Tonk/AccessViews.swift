import HarnessCore
import SwiftUI

/// Small rounded label, e.g. "for agents".
struct LinkBadge: View {
  let text: String
  var body: some View {
    Text(text).font(.caption.weight(.medium)).foregroundStyle(.secondary)
      .padding(.horizontal, 6).padding(.vertical, 1)
      .background(.quaternary, in: Capsule())
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
          .frame(width: 24, height: 24)
          .background(.fill.secondary, in: Circle())
          .background(Color(nsColor: .windowBackgroundColor), in: Circle())
          .overlay { Circle().strokeBorder(Color(nsColor: .separatorColor), lineWidth: 1) }
          .help(member.displayName)
      }
      if members.count > limit {
        Text("+\(members.count - limit)").font(.caption).foregroundStyle(.secondary)
          .padding(.leading, 10)
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
  @ObservedObject var access: AccessModel
  @State private var confirming = false

  var body: some View {
    TimelineView(.periodic(from: .now, by: 60)) { context in
      HStack(alignment: .center, spacing: 12) {
        Image(systemName: link.symbol).foregroundStyle(.secondary)
          .frame(width: 20)
          .accessibilityHidden(true)
        VStack(alignment: .leading, spacing: 2) {
          HStack(spacing: 6) {
            Text(link.title)
            if link.forAgents { LinkBadge(text: "for agents") }
          }
          Text(link.detail(now: context.date)).font(.callout).foregroundStyle(.secondary)
          if let usage = link.usage(madeAt: access.madeAt(link), now: context.date) {
            Text(usage).font(.callout).foregroundStyle(.secondary)
          }
        }
        .lineLimit(1)
        Spacer(minLength: 12)
        actions
      }
      .padding(.vertical, 2)
    }
    .accessibilityElement(children: .contain)
    .confirmationDialog(confirmTitle, isPresented: $confirming) {
      Button(link.canReset ? "Reset Link" : confirmAction, role: .destructive) {
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
    let stored = access.storedLink(link)
    HStack(spacing: 8) {
      if busy { ProgressView().controlSize(.small) }
      Button(link.canReset ? "Reset…" : (link.kind == .publicRead ? "Make Private…" : "Revoke…")) {
        confirming = true
      }
      .disabled(busy || space == nil)
      Button("Copy") { access.copyLink(link) }
        .disabled(busy || stored == nil)
        .help(
          stored == nil
            ? "Only links made on this Mac can be copied. Reset it or make a new link to get one."
            : "Copy link")
    }
    .buttonStyle(.bordered)
  }

  private var confirmTitle: String {
    switch link.kind {
    case .person where link.canReset: "Reset this link?"
    case .publicRead: "Make this space private?"
    default: "Revoke this link?"
    }
  }

  private var confirmAction: String {
    link.kind == .publicRead ? "Make Private" : "Revoke"
  }

  private var confirmMessage: String {
    switch link.kind {
    case .person:
      "A new link is made and copied. The old link stops working; people who already joined keep their access."
    case .scoped:
      "The link stops working. People who already joined keep their access."
    case .agent:
      "Agents using this link lose access to the space."
    case .publicRead:
      "The public address stops working and readers lose access."
    }
  }
}
