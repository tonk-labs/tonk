import HarnessCore
import SwiftUI

struct ContentView: View {
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
  @ObservedObject var model: HarnessModel
  @ObservedObject var runtime: RuntimeModel
  @ObservedObject var access: AccessModel
  /// The hub lists spaces and links; a space window shows one space's view with its chat.
  var isSpace = false
  @Environment(\.openWindow) private var openWindow
  /// Each space window owns its page; the launcher has none.
  var spacePage: SpacePage?
  private var draft: String {
    get { model.saved.conversation.draft ?? "" }
    nonmutating set {
      model.saved.conversation.draft = newValue
      model.persist()
    }
  }
  private var showingChat: Bool {
    get { model.showingChat }
    nonmutating set { model.showingChat = newValue }
  }
  @State private var spaceSearch = ""
  @FocusState private var searchFocused: Bool
  @State private var contactSearchFocusRequest = 0
  @State private var page: DesktopPage?
  @State private var sidebarColumns = NavigationSplitViewVisibility.all
  @State private var newLink: NewLinkRequest?
  @State private var topChromeHovered = false
  @State private var chromePreview = true
  private var chromeVisible: Bool { topChromeHovered || chromePreview }
  /// The chat sidebar opens on its list of chats; choosing one pushes the chat.
  @State private var showHistory = true
  @State private var settingsShowsChat = false
  @State private var accountError: String?
  @State private var signingOut = false
  @FocusState private var composing: Bool

  private var startupFailure: String? {
    if let message = runtime.accountMessage { return message }
    if let error = runtime.error { return error }
    return runtime.catalogError
  }

  private var startupLoading: some View {
    let failure = startupFailure
    return SpaceLoadingView(
      title: failure == nil ? "Opening Tonk…" : "Couldn’t open Tonk",
      detail: failure ?? "Loading your workspace.",
      waiting: failure != nil
    ) {
      if failure != nil {
        Button("Try again") { runtime.load() }
      }
    }
    .background(.background)
  }

  private var workspaceLayers: some View {
    ZStack {
      if isSpace {
        GeometryReader { geometry in
          AnimatedSidebar(
            isVisible: showingChat, reduceMotion: reduceMotion,
            maximumWidth: geometry.size.width / 2,
            sidebar: viewSidebar, detail: viewDetail,
            onVisibilitySettled: { visible in
              if showingChat == visible { composing = visible }
            }
          )
        }
        .overlay(alignment: .top) {
          if !showingChat {
            ViewChromeHoverRegion { topChromeHovered = $0 }
              .frame(maxWidth: .infinity).frame(height: ViewChromeHoverRegion.revealDistance)
              .overlay(alignment: .top) {
                collapsedChrome
                  .opacity(chromeVisible ? 1 : 0)
                  .offset(y: chromeVisible || reduceMotion ? 0 : -16)
                  .animation(reduceMotion ? nil : .easeInOut(duration: 0.22), value: chromeVisible)
                  .allowsHitTesting(chromeVisible)
                  .accessibilityHidden(!chromeVisible)
              }
              .task(id: chromePreview) {
                guard chromePreview else { return }
                do { try await Task.sleep(for: .seconds(3)) } catch { return }
                chromePreview = false
              }
          }
        }
        .background(ViewWindowConfiguration())
        .ignoresSafeArea(.container, edges: .top)
      } else {
        // The account page stays mounted out of sight; the passkey sheet needs it on screen.
        RuntimeView(webView: runtime.webView)
          .opacity(runtime.nativeSignInPending ? 1 : 0)
          .allowsHitTesting(false)
          .accessibilityHidden(true)
        if !runtime.showsSpacePicker {
          if runtime.accountStatusKnown {
            TonkWelcomeView(runtime: runtime)
          } else {
            startupLoading
          }
        } else {
          desktop
        }
      }
    }
  }

  private func focusCurrentSearch() {
    if page == .addressBook { contactSearchFocusRequest += 1 } else { searchFocused = true }
  }

  private var commandTargets: some View {
    workspaceLayers
      .containerBackground(
        reduceTransparency
          ? AnyShapeStyle(Color(nsColor: .windowBackgroundColor)) : AnyShapeStyle(.thinMaterial),
        for: .window
      )
      .focusedSceneValue(
        \.modelSettings,
        signingOut ? nil : openSettings
      )
      .focusedSceneValue(
        \.focusSearch,
        !isSpace && runtime.showsSpacePicker
          ? focusCurrentSearch : nil
      )
      .focusedSceneValue(\.refreshContent, refreshDisabled ? nil : refreshContent)
      .focusedSceneValue(\.newChat, newChatAction)
      .focusedSceneValue(
        \.newLink,
        runtime.spaces.isEmpty || !runtime.accountConnected || signingOut ? nil : startNewLink
      )
      .focusedSceneValue(
        \.toggleAgentSidebar,
        !isSpace || model.openedSpace == nil || signingOut ? nil : toggleAgentSidebar
      )
      .focusedSceneValue(
        \.openSpaceInBrowser,
        !isSpace || model.openedSpace == nil || signingOut ? nil : openSpaceInBrowser
      )
  }

  private var windowContent: some View {
    commandTargets
      .navigationTitle(
        isSpace
          ? (viewTitle ?? model.openedSpace?.title ?? "Space")
          : (runtime.showsSpacePicker
            ? pageTitle : (RuntimeLocation.isChannelHome ? "" : "Tonk"))
      )
      .navigationSubtitle(
        isSpace || RuntimeLocation.isChannelHome
          ? "" : RuntimeLocation.home.absoluteString
      )
      .toolbar { windowToolbar }
      .toolbarBackground(
        reduceTransparency
          ? AnyShapeStyle(Color(nsColor: .textBackgroundColor)) : AnyShapeStyle(.ultraThinMaterial),
        for: .windowToolbar
      )
      .toolbarBackgroundVisibility(isSpace ? .hidden : .visible, for: .windowToolbar)
      .disabled(signingOut)
  }

  var body: some View {
    windowContent
      .alert(
        "Couldn’t sign out",
        isPresented: Binding(
          get: { accountError != nil }, set: { if !$0 { accountError = nil } }
        )
      ) {
        Button("OK") { accountError = nil }
      } message: {
        Text(accountError ?? "")
      }
      .onReceive(NotificationCenter.default.publisher(for: .tonkShowSettings)) { notification in
        if !isSpace {
          settingsShowsChat = notification.object as? Bool ?? false
          page = .account
        }
      }
      .sheet(item: $newLink) { request in
        NewLinkSheet(spaces: runtime.spaces, access: access, subject: request.id)
      }
      .overlay(alignment: .bottom) { noticeToast }
      .onChange(of: showingChat) { _, visible in
        topChromeHovered = false
        if !visible {
          composing = false
          chromePreview = true
        }
      }
      .onChange(of: model.saved.activeSessionID) { _, _ in composing = showingChat }
  }

  private var accountActionsDisabled: Bool {
    model.busy || model.creatingSpace || model.connecting || model.loginPending
      || runtime.signInPending || runtime.catalogLoading
  }

  private var chatActionsDisabled: Bool {
    model.busy || model.connecting || model.creatingSpace || model.loginPending
  }

  @ToolbarContentBuilder
  private var windowToolbar: some ToolbarContent {
    // Links fill in space by space; the page shows placeholders, the toolbar the activity.
    if !isSpace && runtime.showsSpacePicker && page == .allLinks
      && (!access.loadedAll || access.refreshingAll)
    {
      ToolbarItem(id: "links-loading", placement: .primaryAction) {
        ProgressView().controlSize(.small)
          .help("Loading links").accessibilityLabel("Loading links")
      }.customGlassToolbarItem()
    }
    if !isSpace && runtime.showsSpacePicker, let space = selectedSpace {
      ToolbarItem(id: "share-space", placement: .primaryAction) {
        SpaceShareControl(space: space, access: access)
          .disabled(!runtime.accountConnected)
      }.customGlassToolbarItem()
      ToolbarItem(id: "space-sync", placement: .navigation) {
        SpaceSyncControl(runtime: runtime, space: space)
      }.customGlassToolbarItem()
    }
  }

  private var newChatAction: (() -> Void)? {
    guard isSpace, model.openedSpace != nil, !chatActionsDisabled else { return nil }
    return startNewChat
  }

  private var viewTitle: String? {
    guard let space = model.openedSpace else { return nil }
    if ActivityFixture.requested { return "Fixture view" }
    return access.spaces[space.id]?.views.first { $0.url(in: space) == spacePage?.location }?.title
  }

  private var refreshDisabled: Bool {
    runtime.loading || runtime.catalogLoading || runtime.signInPending
  }

  private func refreshContent() {
    if !isSpace {
      Task {
        await runtime.refreshSpaces()
        await access.refreshAll(runtime.spaces)
        await model.syncModelChoice()
      }
    } else {
      spacePage?.reload()
    }
  }

  private func openSpaceInBrowser() {
    guard let space = model.openedSpace else { return }
    NSWorkspace.shared.open(space.url)
  }

  private func toggleAgentSidebar() {
    showingChat.toggle()
  }

  private var chat: some View {
    VStack(spacing: 0) {
      ScrollViewReader { reader in
        ScrollView {
          LazyVStack(alignment: .leading, spacing: 12) {
            ForEach(ChatTranscript.grouped(model.saved.conversation.messages)) { message in
              HStack {
                if message.role == "user" { Spacer(minLength: 24) }
                VStack(alignment: .leading, spacing: 7) {
                  MessageMarkdown(text: message.text).font(.system(size: 13))
                }
                .padding(message.role == "user" ? 10 : 0)
                .background(
                  message.role == "user" ? Color(nsColor: .textBackgroundColor) : .clear,
                  in: RoundedRectangle(cornerRadius: 14))
                if message.role != "user" { Spacer(minLength: 0) }
              }
              if let entries = message.toolActivity, !entries.isEmpty {
                DisclosureGroup("Space activity (\(entries.count))") {
                  ForEach(Array(entries.enumerated()), id: \.offset) { _, entry in
                    Text(entry).font(.caption).foregroundStyle(.secondary)
                      .frame(maxWidth: .infinity, alignment: .leading)
                  }
                }.font(.caption)
              }
            }
            if let proposal = model.saved.conversation.spaceProposal {
              VStack(alignment: .leading, spacing: 10) {
                Text("Create “\(proposal.name)”?").font(.headline)
                Text(proposal.reason).foregroundStyle(.secondary)
                if let error = model.spaceCreationError {
                  Text(error).font(.callout).foregroundStyle(.secondary).textSelection(.enabled)
                }
                if model.creatingSpace {
                  HStack {
                    ProgressView().controlSize(.small)
                    Text("Creating space…")
                  }
                } else {
                  HStack {
                    Button(proposal.submitted ? "Check status" : "Create space") {
                      showingChat = true
                      Task { await model.acceptSpaceProposal() }
                    }.disabled(!model.canSend)
                    Button(proposal.submitted ? "Dismiss" : "Not now") {
                      model.dismissSpaceProposal()
                    }
                    .disabled(model.busy || model.creatingSpace)
                  }
                }
              }.padding(16).frame(maxWidth: .infinity, alignment: .leading)
                .background(.quaternary, in: RoundedRectangle(cornerRadius: 16))
            }
            if model.busy {
              HStack(spacing: 8) {
                ProgressView().controlSize(.small)
                Text(model.activity).font(.caption).foregroundStyle(.secondary)
              }
            } else if model.saved.conversation.lastTurnStatus == "interrupted" {
              Text("Reply stopped").font(.caption).foregroundStyle(.secondary)
            }
            Color.clear.frame(height: 1).id("end")
          }.frame(maxWidth: 680).padding(.horizontal, 6).padding(.vertical, 10).frame(
            maxWidth: .infinity)
        }
        .overlay {
          if model.connecting {
            VStack(spacing: 12) {
              ProgressView().controlSize(.large)
              Text("Starting your agent…").font(.headline)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color(nsColor: .textBackgroundColor))
          } else if model.saved.conversation.messages.isEmpty && !model.busy {
            Text(
              "Ask anything about \(model.openedSpace?.title ?? "this space"), or ask for a change."
            )
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
            .padding(24)
            .allowsHitTesting(false)
          }
        }
        .onChange(of: model.saved.conversation.messages) { _, _ in
          reader.scrollTo("end", anchor: .bottom)
        }
      }

    }
    .background(DesktopStyle.group)
    .safeAreaInset(edge: .bottom, spacing: 0) { composer }
  }

  private var chatTitle: String {
    let first = model.saved.conversation.messages.first { $0.role == "user" }?.text
    return first.map { String($0.replacingOccurrences(of: "\n", with: " ").prefix(80)) }
      ?? "New chat"
  }

  private var viewSidebar: some View {
    VStack(alignment: .leading, spacing: 10) {
      HStack(spacing: 6) {
        ViewWindowControls().frame(width: 56, height: 28).padding(.leading, 6)
        Spacer(minLength: 4)
        ViewChromeButton(title: "New Chat (⇧⌘N)", asset: "r2IconSquarePen", action: startNewChat)
          .disabled(chatActionsDisabled)
        allChatsButton
        viewSidebarToggle
      }.frame(height: 36).background(ViewWindowDragRegion())
      Text(showHistory ? "Chats" : chatTitle).font(.system(size: 13, weight: .semibold))
        .lineLimit(1).help(showHistory ? "Chats" : chatTitle)
        .padding(.leading, 6).accessibilityAddTraits(.isHeader)
      if showHistory {
        ChatHistoryView(
          model: model, space: model.openedSpace, onOpen: openHistory)
      } else {
        chat
      }
    }.padding(.horizontal, 10).padding(.top, 10).padding(.bottom, 8)
      .background(DesktopStyle.group)
  }

  private func startNewChat() {
    model.startSpaceChat()
    showHistory = false
  }

  /// Switches the sidebar between the open chat and the list of chats.
  private var allChatsButton: some View {
    Button {
      if !showHistory { model.saved.checkpointChat() }
      showHistory.toggle()
    } label: {
      DesktopGlyph(name: "r2IconList", size: 15)
    }
    .nativeControl(circular: true).controlSize(.large)
    .background(showHistory ? Color.primary.opacity(0.1) : .clear, in: Circle())
    .help(showHistory ? "Back to chat (⇧⌘H)" : "All chats (⇧⌘H)")
    .accessibilityLabel("All chats")
    .accessibilityAddTraits(showHistory ? .isSelected : [])
    .keyboardShortcut("h", modifiers: [.command, .shift])
    .disabled(chatActionsDisabled)
  }

  private var viewSidebarToggle: some View {
    ViewChromeButton(
      title: showingChat ? "Hide sidebar (⌘B)" : "Show sidebar (⌘B)",
      asset: "r2IconPanelLeft", action: toggleAgentSidebar)
  }

  private var viewMoreMenu: some View {
    Menu {
      Button("Refresh", action: refreshContent).disabled(refreshDisabled)
      Button("Open in Browser", action: openSpaceInBrowser)
      Button("Chat Agent Settings…") { openChatSettings() }
    } label: {
      Label {
        Text("More Options")
      } icon: {
        DesktopGlyph(name: "r2IconMore", size: 15)
      }
      .labelStyle(.iconOnly).frame(width: 28, height: 28)
    }.menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
      .frame(width: 28, height: 28).controlSurface(radius: 14)
      .help("More Options")
      .accessibilityLabel("More Options")
  }

  /// The space and view as one breadcrumb; long names truncate rather than wrap.
  private var viewAddress: some View {
    HStack(spacing: 7) {
      if let space = model.openedSpace {
        SpaceSyncControl(runtime: runtime, space: space)
        Button(space.title) { openWindow(id: "main") }.buttonStyle(.plain)
          .fontWeight(.semibold).lineLimit(1).help("\(space.title): show spaces and links")
        DesktopGlyph(name: "r2IconChevronRight", size: 12).foregroundStyle(.secondary)
        Text(viewTitle ?? "Home").foregroundStyle(.secondary).lineLimit(1)
          .help(viewTitle ?? "Home")
      }
      Spacer(minLength: 0)
    }.font(.system(size: 13)).frame(height: 32)
  }

  /// With the chat open, the space's own controls sit on the view card, so the
  /// chat sidebar holds only chat.
  private var cardHeader: some View {
    HStack(spacing: 8) {
      viewAddress
      if let space = model.openedSpace {
        SpaceShareControl(space: space, access: access, compact: true, surface: .workspace)
      }
      viewMoreMenu
    }.padding(.leading, 14).padding(.trailing, 8).frame(height: 44)
      .background(Color(nsColor: .windowBackgroundColor))
      .overlay(alignment: .bottom) { Divider() }
  }

  private var viewDetail: some View {
    VStack(spacing: 0) {
      if showingChat { cardHeader }
      workspace
    }
    .clipShape(RoundedRectangle(cornerRadius: showingChat ? 12 : 0))
    .padding(.vertical, showingChat ? 8 : 0).padding(.trailing, showingChat ? 8 : 0)
    .background(DesktopStyle.group)
  }

  private var collapsedChrome: some View {
    HStack(spacing: 10) {
      ViewWindowControls().frame(width: 56, height: 28)
      viewSidebarToggle
      viewAddress.padding(.horizontal, 10).frame(maxWidth: 340)
      Spacer(minLength: 0)
      if let space = model.openedSpace {
        SpaceShareControl(space: space, access: access, surface: .workspace)
      }
      viewMoreMenu
    }.padding(.horizontal, 16).frame(height: 52)
      .background(.bar).background(Color(nsColor: .windowBackgroundColor))
      .background(ViewWindowDragRegion())
      .shadow(color: .black.opacity(0.1), radius: 5, y: 3)
      .contentShape(Rectangle())
  }

  private func openHistory(_ session: ChatSession) {
    if session.id == model.saved.activeSessionID {
      showHistory = false
      return
    }
    Task {
      if await model.openChat(session.id, connectInBackground: true) {
        showingChat = true
        showHistory = false
      }
    }
  }

  /// A quick model switch in place; the full settings stay one item away.
  private var composerModel: some View {
    Menu {
      if !composerModels.isEmpty {
        Picker(
          "Model",
          selection: Binding(get: { model.connection.model }, set: { chooseModel($0) })
        ) {
          ForEach(composerModels, id: \.id) { item in Text(item.name).tag(item.id) }
        }.pickerStyle(.inline)
        Divider()
      }
      Button("Chat Agent Settings…") { openChatSettings() }
    } label: {
      Text(model.connecting ? "Connecting…" : model.modelLabel).lineLimit(1)
        .font(.system(size: 12))
    }
    .menuStyle(.button).nativeControl().fixedSize()
    .help("Model: \(model.modelLabel)")
    .accessibilityLabel("Model: \(model.modelLabel)")
    .disabled(accountActionsDisabled)
    .task(id: model.signedIn) {
      guard model.signedIn, composerModels.count <= 1 else { return }
      if model.provider == .chatGPT { await model.refreshSubscriptionModels() }
      if model.provider == .claude { await model.refreshClaudeModels() }
    }
  }

  /// The signed-in subscription's models, led by its default; other providers take a typed ID.
  private var composerModels: [(id: String, name: String)] {
    guard model.signedIn else { return [] }
    switch model.provider {
    case .chatGPT:
      return [("", "Default from ChatGPT")] + model.subscriptionModels.map { ($0.id, $0.name) }
    case .claude:
      return [("", "Default from Claude Code")]
        + model.claudeModels.filter { $0.id != "default" }.map { ($0.id, $0.name) }
    default:
      return []
    }
  }

  private func chooseModel(_ id: String) {
    guard id != model.connection.model else { return }
    var next = model.connection
    next.model = id
    Task {
      do {
        try await model.configureProvider(
          model.provider, connection: next, key: "", codexExecutable: model.saved.codexExecutable,
          surface: .workspace)
      } catch { model.error = error.localizedDescription }
    }
  }

  private var canSubmit: Bool {
    model.canSend && !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
  }

  private var composer: some View {
    VStack(spacing: 0) {
      if let error = model.error {
        HStack(alignment: .top) {
          Image(systemName: "exclamationmark.circle")
          Text(error).font(.callout).textSelection(.enabled).lineSpacing(4)
          Spacer(minLength: 0)
          Button {
            model.error = nil
          } label: {
            Image(systemName: "xmark")
          }.buttonStyle(.plain).accessibilityLabel("Dismiss error")
        }.padding(12).background(
          Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 12)
        ).padding(.horizontal, 16).padding(
          .bottom, 8)
      }

      if ![.chatGPT, .claude].contains(model.provider) && !model.signedIn {
        Button("Configure model") { openChatSettings() }.padding(20)
      } else if model.connected && !model.signedIn {
        VStack(spacing: 10) {
          if model.loginPending {
            ProgressView("Continue in your browser")
            Button("Cancel sign-in") { Task { await model.cancelLogin() } }
          } else {
            Button(model.provider == .claude ? "Sign in with Claude" : "Sign in with ChatGPT") {
              Task { await model.signIn() }
            }.buttonStyle(.borderedProminent)
            Text(
              model.provider == .claude
                ? "Uses your Claude Code sign-in on this Mac."
                : "Tonk keeps its own sign-in on this Mac."
            ).font(.caption).foregroundStyle(
              .secondary)
          }
        }.frame(maxWidth: .infinity).padding(20)
      } else {
        VStack(alignment: .leading, spacing: 6) {
          TextField(
            "Ask about \(model.openedSpace?.title ?? "this space")…",
            text: Binding(get: { draft }, set: { draft = $0 }),
            axis: .vertical
          )
          .textFieldStyle(.plain).lineLimit(1...8).focused($composing)
          .frame(minHeight: 20, alignment: .center)
          .onSubmit { submit() }.disabled(!model.canSend)
          .accessibilityLabel("Message")
          HStack {
            composerModel
            Spacer(minLength: 4)
            if model.busy {
              Button {
                Task { await model.stopTurn() }
              } label: {
                Image(systemName: "stop.fill").frame(width: 20, height: 20)
              }
              .nativeControl(circular: true).disabled(!model.canStop).accessibilityLabel(
                "Stop reply")
            } else {
              Button(action: submit) {
                DesktopGlyph(name: "r2IconArrowUp", size: 14)
              }
              .nativeControl(prominent: true, circular: true)
              .disabled(!canSubmit)
              .accessibilityLabel("Send message")
            }
          }
        }.controlSize(.small).padding(10)
          .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 16))
          .overlay(
            RoundedRectangle(cornerRadius: 16).strokeBorder(.primary.opacity(0.08), lineWidth: 0.5)
          )
          .padding(.top, 6)
      }
    }
  }

  @ViewBuilder private var workspace: some View {
    if let spacePage { SpaceWorkspace(page: spacePage, viewTitle: viewTitle) } else { Color.clear }
  }

  private var pageTitle: String {
    switch page {
    case .addressBook: "Address book"
    case .account: "Settings"
    case .allLinks: "All links"
    case .space(let id): runtime.spaces.first(where: { $0.id == id })?.title ?? "Spaces"
    case nil: "Spaces"
    }
  }

  private var selectedSpace: TonkSpace? {
    guard case .space(let id) = page else { return nil }
    return runtime.spaces.first(where: { $0.id == id })
  }

  private func startNewLink() {
    guard let first = (isSpace ? model.openedSpace : selectedSpace) ?? runtime.spaces.first
    else { return }
    newLink = NewLinkRequest(id: first.id)
  }

  private func openSettings() { showSettings(chat: false) }

  private func openChatSettings() { showSettings(chat: true) }

  private func showSettings(chat: Bool) {
    if isSpace {
      openWindow(id: "main")
      DispatchQueue.main.async {
        NotificationCenter.default.post(name: .tonkShowSettings, object: chat)
      }
    } else {
      settingsShowsChat = chat
      page = .account
    }
  }

  private func signOutOfTonk() {
    signingOut = true
    Task {
      defer { signingOut = false }
      do {
        try await runtime.signOut()
        access.reset()
        model.openedSpace = nil
        showingChat = false
        page = nil
      } catch { accountError = error.localizedDescription }
    }
  }

  /// Opening the same space view again brings its existing window forward.
  private func open(_ view: SpaceViewEntry, in space: TonkSpace) {
    openWindow(value: SpaceWindowRequest(subject: space.subject, url: view.url(in: space)))
    Telemetry.shared.record(.workspace, .openView, surface: .hub)
  }

  private var sidebarToggle: some View {
    let visible = sidebarColumns != .detailOnly
    return Button {
      withAnimation { sidebarColumns = visible ? .detailOnly : .all }
    } label: {
      Image(systemName: "sidebar.left").frame(width: 20, height: 20)
    }
    .nativeControl(circular: true).controlSize(.large)
    .keyboardShortcut("s", modifiers: [.command, .control])
    .accessibilityLabel(visible ? "Hide sidebar" : "Show sidebar")
    .help(visible ? "Hide sidebar (⌃⌘S)" : "Show sidebar (⌃⌘S)")
  }

  private var desktop: some View {
    NavigationSplitView(columnVisibility: $sidebarColumns) {
      DesktopSidebar(
        runtime: runtime, access: access, selection: $page, search: $spaceSearch,
        searchFocused: $searchFocused,
        onNewLink: startNewLink,
        onSettings: openSettings,
        onSignOut: runtime.accountConnected && RuntimeLocation.deployment != .local
          ? signOutOfTonk : nil
      )
      .frame(minWidth: 192, idealWidth: 192)
      .navigationSplitViewColumnWidth(min: 192, ideal: 192, max: 240)
      // Replaced by a circular toggle that matches the window's other controls.
      .toolbar(removing: .sidebarToggle)
      .toolbar {
        // Trailing edge of the sidebar, where the system toggle sat. It stays
        // in the titlebar when the sidebar collapses, so one toggle serves both.
        if #available(macOS 26.0, *) {
          ToolbarSpacer(.flexible)
        }
        ToolbarItem(id: "toggle-sidebar", placement: .automatic) { sidebarToggle }
          .customGlassToolbarItem()
      }
    } detail: {
      Group {
        switch page {
        case .space:
          if let space = selectedSpace {
            SpaceOverviewView(
              space: space, access: access,
              canRefresh: runtime.accountConnected && !runtime.showingCachedCatalog,
              onOpenView: { open($0, in: space) },
              onNewLink: { Task { await access.make(.person, in: space) } })
          } else {
            CatalogStatusView(runtime: runtime)
          }
        case .allLinks:
          LinksPageView(
            agentsOnly: false, spaces: runtime.spaces, access: access, onNewLink: startNewLink)
        case .addressBook:
          AddressBookView(
            access: access, spaces: runtime.spaces,
            searchFocusRequest: contactSearchFocusRequest,
            onOpenSpace: { page = .space($0.id) })
        case .account:
          DesktopSettingsView(
            model: model, access: access, onSignOut: signOutOfTonk, showsChat: settingsShowsChat)
        case nil:
          CatalogStatusView(runtime: runtime)
        }
      }
      .background(.background)
    }
    .disabled(model.busy || model.connecting || model.creatingSpace || runtime.pendingSpace != nil)
    .onChange(of: runtime.spaces.map(\.id), initial: true) { _, ids in
      if runtime.catalogLoaded { access.setCatalog(runtime.spaces) }
      // All links is home; a space that leaves the account falls back to it.
      if case .space(let id) = page, ids.contains(id) { return }
      if page == nil, let fixture = ActivityFixture.initialPage(runtime.spaces) {
        page = fixture
      } else if page == nil
        || page.map({ if case .space = $0 { true } else { false } }) == true
      {
        page = .allLinks
      }
    }
    .onChange(of: runtime.accountConnected) { _, connected in
      // Nothing learned under one account may show under the next.
      if !connected { access.reset() }
    }
    // Analytics identity follows the signed-in profile: the same hashed id the web uses.
    .task(id: "\(runtime.accountConnected)-\(runtime.accountEpoch)") {
      guard runtime.accountConnected, Telemetry.shared.isEnabled else {
        Telemetry.shared.identify(profileDID: nil)
        return
      }
      let identity = try? await runtime.accountScript("return await api('/api/identify');")
      Telemetry.shared.identify(profileDID: identity?["did"] as? String)
    }
    .task(id: runtime.catalogLoaded && runtime.accountConnected ? runtime.spaces.map(\.id) : nil) {
      guard runtime.catalogLoaded && runtime.accountConnected else { return }
      await access.refreshAll(runtime.spaces)
    }
    .task(id: runtime.catalogLoaded && runtime.accountConnected && !runtime.showingCachedCatalog) {
      guard runtime.catalogLoaded && runtime.accountConnected && !runtime.showingCachedCatalog
      else { return }
      await model.syncModelChoice()
    }
  }

  @ViewBuilder private var noticeToast: some View {
    if let notice = access.notice {
      Text(notice)
        .padding(.horizontal, 16).padding(.vertical, 10)
        .controlSurface()
        .padding(.bottom, 24)
        .transition(.opacity)
        .task(id: notice) {
          do { try await Task.sleep(for: .seconds(4)) } catch { return }
          if access.notice == notice { access.notice = nil }
        }
        .accessibilityAddTraits(.updatesFrequently)
    }
  }

  private func submit() {
    guard showingChat, model.canSend, !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    else {
      return
    }
    let text = draft
    draft = ""
    Task { await model.send(text) }
  }
}

/// Which space the New link sheet starts on.
struct NewLinkRequest: Identifiable {
  let id: String
}

extension Notification.Name {
  static let tonkShowSettings = Notification.Name("tonk.showSettings")
}
