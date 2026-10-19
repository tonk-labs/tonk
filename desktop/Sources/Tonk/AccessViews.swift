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

/// A space's people: a summary and overlapping initials. Hovering an initial
/// grows it into a pill with the member's full name; past the limit, a "+N"
/// chip opens a list of everyone.
struct Roster: View {
  let summary: String
  let members: [SpaceMember]
  var limit = 6
  @State private var hovered: SpaceMember.ID?
  @State private var listShown = false
  @State private var overMore = false
  @State private var overList = false

  var body: some View {
    HStack(spacing: 10) {
      Text(summary)
      HStack(spacing: -6) {
        ForEach(members.prefix(limit)) { chip($0) }
        if members.count > limit { more }
      }
      .accessibilityElement(children: .contain)
    }
    .animation(.spring(duration: 0.35, bounce: 0.2), value: hovered)
  }

  private func chip(_ member: SpaceMember) -> some View {
    let open = hovered == member.id
    return HStack(spacing: 4) {
      Text(member.initial).font(.caption.weight(.semibold))
      if open {
        Text(member.displayName).font(.caption).lineLimit(1).fixedSize()
          .transition(.opacity.combined(with: .move(edge: .leading)))
      }
    }
    .padding(.horizontal, open ? 8 : 0)
    .rosterChip()
    .zIndex(open ? 1 : 0)
    .onHover { over in
      if over {
        hovered = member.id
      } else if hovered == member.id {
        hovered = nil
      }
    }
    .accessibilityElement(children: .ignore)
    .accessibilityLabel(member.displayName)
  }

  /// Opens on hover and stays open while the pointer is over the chip or the list;
  /// a click opens it too, so it also works from the keyboard.
  private var more: some View {
    Button {
      listShown.toggle()
    } label: {
      Text("+\(members.count - limit)").font(.caption.weight(.semibold))
        .padding(.horizontal, 4)
        .rosterChip()
    }
    .buttonStyle(.plain)
    .onHover { overMore = $0; settleList() }
    .popover(isPresented: $listShown, arrowEdge: .bottom) {
      MemberList(members: members)
        .onHover { overList = $0; settleList() }
    }
    .accessibilityLabel("Show all \(members.count) members")
  }

  private func settleList() {
    if overMore || overList {
      listShown = true
      return
    }
    // A short grace lets the pointer cross from the chip into the popover.
    Task {
      try? await Task.sleep(for: .milliseconds(250))
      if !overMore && !overList { listShown = false }
    }
  }
}

/// Everyone in a space, one initial and name per row.
private struct MemberList: View {
  let members: [SpaceMember]

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 8) {
        ForEach(members) { member in
          HStack(spacing: 8) {
            Text(member.initial).font(.caption.weight(.semibold)).rosterChip()
              .accessibilityHidden(true)
            Text(member.displayName).lineLimit(1)
          }
        }
      }
      .padding(12)
    }
    .frame(minWidth: 180, maxHeight: 320)
    .fixedSize(horizontal: true, vertical: false)
  }
}

extension View {
  /// The roster's initial circle, which grows into a capsule around wider content.
  fileprivate func rosterChip() -> some View {
    frame(minWidth: 24, minHeight: 24)
      .background(.fill.secondary, in: Capsule())
      .background(Color(nsColor: .windowBackgroundColor), in: Capsule())
      .clipShape(Capsule())
      .overlay { Capsule().strokeBorder(Color(nsColor: .separatorColor), lineWidth: 1) }
      .contentShape(Capsule())
  }
}

/// The kind of link at a glance: a bot for agents, a globe for public, else a link.
struct LinkKindIcon: View {
  let kind: AccessLinkKind

  var body: some View {
    switch kind {
    case .agent: BotGlyph()
    case .publicRead: Image(systemName: "globe")
    case .person, .scoped: Image(systemName: "link")
    }
  }
}

/// One link: what it allows, who used it, and Reset / Revoke / Copy.
struct AccessLinkRow: View {
  let link: AccessLink
  let space: TonkSpace?
  @ObservedObject var access: AccessModel
  @State private var confirming = false
  @State private var resetting = false

  var body: some View {
    TimelineView(.periodic(from: .now, by: 60)) { context in
      HStack(alignment: .center, spacing: 12) {
        LinkKindIcon(kind: link.kind).foregroundStyle(.secondary)
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
      Button(resetting ? "Reset Link" : confirmAction, role: .destructive) {
        guard let space else { return }
        Task {
          if resetting {
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
      if link.canReset {
        Button("Reset…") {
          resetting = true
          confirming = true
        }
        .disabled(busy || space == nil)
      }
      Button(link.kind == .publicRead ? "Make Private…" : "Revoke…") {
        resetting = false
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
    resetting
      ? "Reset this link?"
      : (link.kind == .publicRead ? "Make this space private?" : "Revoke this link?")
  }

  private var confirmAction: String {
    link.kind == .publicRead ? "Make Private" : "Revoke"
  }

  private var confirmMessage: String {
    switch link.kind {
    case .person, .scoped:
      resetting
        ? "A new link is made and copied. The old link stops working. People who joined through it may lose access and need the new link."
        : "The link stops working. People who joined through it may lose access unless they have another grant."
    case .agent:
      "Agents using this link lose access to the space."
    case .publicRead:
      "The public address stops working and readers lose access."
    }
  }
}

/// A robot head for agent connections; SF Symbols has no public bot glyph.
struct BotGlyph: View {
  var body: some View {
    BotShape()
      .stroke(style: StrokeStyle(lineWidth: 1.5, lineCap: .round, lineJoin: .round))
      .frame(width: 17, height: 17)
      .accessibilityHidden(true)
  }
}

/// Drawn on a 24-point grid and scaled to fit.
private struct BotShape: Shape {
  func path(in rect: CGRect) -> Path {
    let s = min(rect.width, rect.height) / 24
    func p(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
      CGPoint(x: rect.midX + (x - 12) * s, y: rect.midY + (y - 12) * s)
    }
    var path = Path()
    path.addRoundedRect(
      in: CGRect(origin: p(4, 8), size: CGSize(width: 16 * s, height: 12 * s)),
      cornerSize: CGSize(width: 2 * s, height: 2 * s))
    for (from, to) in [
      ((12, 8), (12, 4)), ((12, 4), (8, 4)),  // antenna
      ((2, 14), (4, 14)), ((20, 14), (22, 14)),  // ears
      ((9, 13), (9, 15)), ((15, 13), (15, 15)),  // eyes
    ] as [((CGFloat, CGFloat), (CGFloat, CGFloat))] {
      path.move(to: p(from.0, from.1))
      path.addLine(to: p(to.0, to.1))
    }
    return path
  }
}
