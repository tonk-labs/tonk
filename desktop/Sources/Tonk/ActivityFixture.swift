import AppKit
import HarnessCore
import SwiftUI

/// Explicit visual fixture. Never loads an account, a provider or existing app data.
@MainActor
enum ActivityFixture {
  static let space = try? JSONDecoder().decode(
    TonkSpace.self,
    from: Data(#"{"subject":"did:key:zFixture","name":"Activity layout fixture"}"#.utf8))
  static var requested: Bool { ProcessInfo.processInfo.arguments.contains("--activity-fixture") }

  static var compactWindow: Bool {
    requested && ProcessInfo.processInfo.arguments.contains("--compact-window-fixture")
  }

  static func prepare(model: HarnessModel, page: SpacePage) {
    let args = ProcessInfo.processInfo.arguments
    guard RuntimeLocation.deployment == .local, args.contains("--data-dir"),
      args.contains("--web-data-id"), model.saved.conversation.messages.isEmpty
    else {
      NSApp.terminate(nil)
      return
    }
    guard let space else { return }
    model.saved.conversation.space = space
    model.saved.conversation.messages = [
      ChatMessage(id: "first", role: "user", text: "Create a checklist."),
      ChatMessage(
        id: "reply-one", role: "assistant", text: "Created three tasks and checked the result."),
      ChatMessage(id: "second", role: "user", text: "Check the filters."),
      ChatMessage(
        id: "reply-two", role: "assistant",
        text: "The read completed. The update could not be confirmed; query before retrying."),
    ]
    model.saved.conversation.appendToolActivity("Reading local space", messageID: "first")
    model.saved.conversation.appendToolActivity("Updating local space", messageID: "first")
    model.saved.conversation.appendToolActivity(
      "Inspecting attached space preview", messageID: "second")
    model.saved.conversation.appendToolActivity(
      "Evaluation could not be confirmed. Do not repeat it; query the space first.",
      messageID: "second")
    if let index = args.firstIndex(of: "--appearance"), args.indices.contains(index + 1) {
      NSApp.appearance = NSAppearance(named: args[index + 1] == "dark" ? .darkAqua : .aqua)
    }
    model.openedSpace = space
    model.showingChat = true
    // Keep the fixture's static page separate from the production navigation policy.
    page.webView.navigationDelegate = nil
    page.showFixture(
      space,
      html:
        "<style>body{font:13px system-ui;padding:24px;color:#777}</style><p>Layout fixture only. No live worker or model.</p>",
      loading: args.contains("--loading-fixture"))
  }
}

extension ActivityFixture {
  /// Links-first layout fixture: invented spaces, people and links, no worker.
  static func prepareLinks(runtime: RuntimeModel, access: AccessModel) {
    let args = ProcessInfo.processInfo.arguments
    guard RuntimeLocation.deployment == .local, args.contains("--data-dir"),
      args.contains("--web-data-id")
    else {
      NSApp.terminate(nil)
      return
    }
    if let index = args.firstIndex(of: "--appearance"), args.indices.contains(index + 1) {
      NSApp.appearance = NSAppearance(named: args[index + 1] == "dark" ? .darkAqua : .aqua)
    }
    let names = ["tonk-team", "kanoodel", "tea-inventory", "little-writer"]
    let subjects = [
      "did:key:z6MkTeam", "did:key:z6MkKanood3", "did:key:z6MkTea", "did:key:z6MkWriter",
    ]
    let catalog = zip(subjects, names).map { #"{"subject":"\#($0)","name":"\#($1)"}"# }
    guard
      let spaces = try? TonkSpace.decodeCatalog(
        Data(("[" + catalog.joined(separator: ",") + "]").utf8))
    else { return }
    let now = Date()
    let team = subjects[0]
    let member = { (did: String, name: String, role: String, via: String?) in
      SpaceMember(did: did, name: name, role: role, invitation: via)
    }
    let members = [
      member("did:key:zKai", "Kai", "tonk:founder", nil),
      member("did:key:zEileen", "Eileen", "tonk:member", nil),
      member("did:key:zJonas", "Jonas", "tonk:member", "tonk:design"),
      member("did:key:zMira", "Mira", "tonk:member", "tonk:plain"),
      member("did:key:zTheo", "Theo", "tonk:member", "tonk:design"),
      member("did:key:zLu", "Lu", "tonk:member", "tonk:plain"),
    ]
    let agents = [
      AgentConnection(
        id: "cycle", subject: team,
        expiresAt: now.addingTimeInterval(7 * 86_400 - 60)
          .timeIntervalSince1970),
      AgentConnection(
        id: "whole", subject: team,
        expiresAt: now.addingTimeInterval(80 * 86_400)
          .timeIntervalSince1970, confirmed: true,
        installations: [.init(id: "i", name: "Eileen’s Claude Code")]),
      AgentConnection(
        id: "kai", subject: team,
        expiresAt: now.addingTimeInterval(40 * 86_400)
          .timeIntervalSince1970, confirmed: true,
        installations: [.init(id: "k", name: "Kai’s Claude Code")]),
      AgentConnection(
        id: "kanoodel", subject: subjects[1],
        expiresAt: now.addingTimeInterval(20 * 86_400)
          .timeIntervalSince1970),
    ]
    let invites = [
      InviteSummary(targetCid: "plain", kind: "open", invitation: "tonk:plain"),
      InviteSummary(targetCid: "design", kind: "open", invitation: "tonk:design"),
    ]
    var fixture: [String: SpaceAccess] = [:]
    for (index, subject) in subjects.enumerated() {
      let roster = index == 0 ? members : [members[1]]
      let isPublic = index == 2
      fixture[subject] = SpaceAccess(
        members: roster,
        links: SpaceAccess.links(
          subject: subject, invites: index == 0 ? invites : [], agents: agents, members: roster,
          isPublic: isPublic),
        agents: agents.filter { $0.subject == subject },
        views: index == 0
          ? [
            SpaceViewEntry(model: "id:board", title: "Board", isHome: true),
            SpaceViewEntry(model: "id:cycle-14", title: "Cycle 14", isHome: false),
            SpaceViewEntry(model: "id:design-notes", title: "Design notes", isHome: false),
          ] : [SpaceViewEntry(model: "tonk/space", title: "Home", isHome: true)],
        isPublic: isPublic, selfDid: "did:key:zEileen")
    }
    var journal = AccessJournal()
    journal.record(
      AccessEvent(
        date: now.addingTimeInterval(-2 * 86_400), subject: team, action: .made, kind: .person,
        target: "design"))
    journal.record(
      AccessEvent(
        date: now.addingTimeInterval(-3 * 3600), subject: team, action: .reset, kind: .person,
        target: "plain"))
    journal.record(
      AccessEvent(
        date: now.addingTimeInterval(-5), subject: team, action: .made, kind: .agent,
        target: "cycle"))
    access.showFixture(
      fixture, agents: agents, accountName: "Eileen Wagner", journal: journal,
      email: "eileen@example.test",
      passkeys: [
        AccountPasskey(
          id: "fixture-one", createdOn: "iCloud Keychain on this Mac",
          createdAt: now.addingTimeInterval(-18 * 86_400)),
        AccountPasskey(
          id: "fixture-two", createdOn: "Firefox on macOS",
          createdAt: now.addingTimeInterval(-39 * 86_400)),
      ])
    runtime.spaces = spaces
    runtime.catalogLoaded = true
    runtime.accountConnected = true
    runtime.accountStatusKnown = true
    runtime.loading = false
    runtime.webView.loadHTMLString(
      "<p>Layout fixture only. No live worker or model.</p>", baseURL: nil)
  }
}

extension ActivityFixture {
  /// Fixture-only: write the window's own drawing to `--snapshot <path>` and quit.
  /// Needs no screen-recording permission, so layout checks can run headless.
  static func snapshotIfRequested(runtime: RuntimeModel, access: AccessModel) {
    let args = ProcessInfo.processInfo.arguments
    if let index = args.firstIndex(of: "--snapshot-sidebar"), args.indices.contains(index + 1) {
      snapshotSidebar(runtime: runtime, access: access, path: args[index + 1])
    }
    guard RuntimeLocation.deployment == .local, let index = args.firstIndex(of: "--snapshot"),
      args.indices.contains(index + 1)
    else { return }
    let path = args[index + 1]
    Task { @MainActor in
      try? await Task.sleep(for: .seconds(3))
      guard let view = NSApp.windows.first(where: \.isVisible)?.contentView?.superview,
        let layer = view.layer
      else { return }
      // Layer rendering includes layer-drawn rows that cacheDisplay skips.
      let scale = view.window?.backingScaleFactor ?? 2
      let width = Int(view.bounds.width * scale)
      let height = Int(view.bounds.height * scale)
      guard
        let context = CGContext(
          data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
          space: CGColorSpace(name: CGColorSpace.sRGB)!,
          bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
      else { return }
      context.setFillColor(NSColor.windowBackgroundColor.cgColor)
      context.fill(CGRect(x: 0, y: 0, width: width, height: height))
      context.scaleBy(x: scale, y: scale)
      layer.render(in: context)
      guard let image = context.makeImage() else { return }
      try? NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])?
        .write(to: URL(fileURLWithPath: path))
      NSApp.terminate(nil)
    }
  }
}

extension ActivityFixture {
  /// Fixture-only first page: `--fixture-page all|address-book|account|<space name>`.
  static func initialPage(_ spaces: [TonkSpace]) -> DesktopPage? {
    let args = ProcessInfo.processInfo.arguments
    guard RuntimeLocation.deployment == .local, let index = args.firstIndex(of: "--fixture-page"),
      args.indices.contains(index + 1)
    else { return nil }
    switch args[index + 1] {
    case "address-book": return .addressBook
    case "account": return .account
    case "all": return .allLinks
    case let name: return spaces.first(where: { $0.title == name }).map { .space($0.id) }
    }
  }
}

extension ActivityFixture {
  /// The split view's sidebar is a separate glass surface that offscreen
  /// rendering misses, so draw the sidebar view by itself.
  fileprivate static func snapshotSidebar(runtime: RuntimeModel, access: AccessModel, path: String)
  {
    struct Host: View {
      let runtime: RuntimeModel
      let access: AccessModel
      @FocusState var focused: Bool
      var body: some View {
        DesktopSidebar(
          runtime: runtime, access: access,
          selection: .constant(runtime.spaces.last.map { .space($0.id) }),
          search: .constant(""), searchFocused: $focused, onNewLink: {},
          onSettings: {}, onSignOut: {}
        )
        .frame(width: 260, height: 760)
        .background(Color(nsColor: .windowBackgroundColor))
      }
    }
    Task { @MainActor in
      try? await Task.sleep(for: .seconds(1))
      let view = NSHostingView(rootView: Host(runtime: runtime, access: access))
      view.frame = CGRect(x: 0, y: 0, width: 260, height: 760)
      let window = NSWindow(
        contentRect: view.frame, styleMask: [.borderless], backing: .buffered, defer: false)
      window.contentView = view
      view.layoutSubtreeIfNeeded()
      try? await Task.sleep(for: .milliseconds(500))
      guard let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return }
      view.cacheDisplay(in: view.bounds, to: bitmap)
      try? bitmap.representation(using: .png, properties: [:])?
        .write(to: URL(fileURLWithPath: path))
    }
  }
}
