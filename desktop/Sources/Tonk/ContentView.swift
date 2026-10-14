import HarnessCore
import SwiftUI

struct ContentView: View {
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
  @ObservedObject var model: HarnessModel
  @ObservedObject var runtime: RuntimeModel
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
  @State private var searchFocusRequest = 0
  @State private var showHistory = false
  @State private var showProviderSettings = false
  @State private var accountError: String?
  @State private var signingOut = false
  @State private var sharingSpace: TonkSpace?
  @State private var syncStatusSpace: TonkSpace?
  @FocusState private var composing: Bool

  private var windowContent: some View {
    ZStack {
      GeometryReader { geometry in
        AnimatedSidebar(
          isVisible: showingChat, reduceMotion: reduceMotion,
          maximumWidth: geometry.size.width / 2,
          sidebar: chat, detail: workspace,
          onVisibilitySettled: { visible in
            if showingChat == visible { composing = visible }
          }
        )
      }
      .modifier(RuntimeToolbarBackground(enabled: !reduceTransparency))
      .opacity(model.openedSpace != nil || runtime.nativeSignInPending ? 1 : 0)
      .allowsHitTesting(model.openedSpace != nil)
      .accessibilityHidden(model.openedSpace == nil)
      if model.openedSpace == nil {
        if !runtime.showsSpacePicker {
          if runtime.accountStatusKnown {
            TonkWelcomeView(runtime: runtime)
          } else {
            let failure = runtime.accountMessage ?? runtime.error ?? runtime.catalogError
            SpaceLoadingView(
              title: failure == nil ? "Opening Tonk…" : "Couldn’t open Tonk",
              detail: failure ?? "Loading your workspace.",
              waiting: failure != nil
            ) {
              if failure != nil {
                Button("Try again") { runtime.load() }.nativeControl()
              }
            }
            .background(.background)
          }
        } else {
          SpacePickerView(runtime: runtime, search: $spaceSearch) { space in
            Task {
              guard let space = await runtime.spaceForOpening(space) else { return }
              if await model.enterSpace(space) {
                runtime.openSpace(space)
                model.openedSpace = space
                showingChat = false
              }
            }
          }.disabled(model.busy || model.connecting || model.creatingSpace || runtime.pendingSpace != nil)
        }
      }
    }
    .nativeControl()
    .containerBackground(
      reduceTransparency
        ? AnyShapeStyle(Color(nsColor: .windowBackgroundColor)) : AnyShapeStyle(.thinMaterial),
      for: .window
    )
    .focusedSceneValue(
      \.modelSettings, accountActionsDisabled || signingOut ? nil : { showProviderSettings = true }
    )
    .focusedSceneValue(
      \.focusSearch,
      model.openedSpace == nil && runtime.showsSpacePicker
        ? { searchFocusRequest += 1 } : nil
    )
    .focusedSceneValue(\.refreshContent, refreshDisabled ? nil : refreshContent)
    .focusedSceneValue(
      \.toggleAgentSidebar,
      model.openedSpace == nil || signingOut ? nil : toggleAgentSidebar
    )
    .focusedSceneValue(
      \.openSpaceInBrowser, model.openedSpace == nil || signingOut ? nil : openSpaceInBrowser
    )
    .navigationTitle(
      model.openedSpace?.title
        ?? (runtime.showsSpacePicker
          ? "Spaces" : (RuntimeLocation.deployment == .production ? "" : "Tonk"))
    )
    .navigationSubtitle(
      RuntimeLocation.deployment == .production ? "" : RuntimeLocation.home.absoluteString
    )
    .toolbar { windowToolbar }
    .toolbarBackground(
      reduceTransparency
        ? AnyShapeStyle(Color(nsColor: .textBackgroundColor)) : AnyShapeStyle(.ultraThinMaterial),
      for: .windowToolbar
    )
    .toolbarBackgroundVisibility(.visible, for: .windowToolbar)
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
    .sheet(isPresented: $showProviderSettings) {
      ProviderSettingsView(model: model)
    }
    .sheet(item: $syncStatusSpace) { space in
      SyncStatusView(runtime: runtime, space: space)
    }
    .sheet(item: $sharingSpace) { space in
      ShareSpaceView(runtime: runtime, space: space)
    }
    .sheet(isPresented: $showHistory) {
      ChatHistoryView(
        model: model, space: model.openedSpace
      ) { session in
        Task {
          if await model.openChat(session.id) {
            if let space = model.saved.conversation.space {
              runtime.openSpace(space)
              model.openedSpace = space
            } else {
              runtime.showSpaces()
            }
            showingChat = true
            showHistory = false
          }
        }
      }
    }
    .onChange(of: showingChat) { _, visible in
      if !visible { composing = false }
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
    if model.openedSpace != nil {
      ToolbarItem(id: "toggle-chat", placement: .navigation) {
        Button {
          toggleAgentSidebar()
        } label: {
          Image(systemName: "sidebar.left").frame(width: 20, height: 20)
        }.nativeControl(circular: true).controlSize(.large)
          .accessibilityLabel(showingChat ? "Hide chat" : "Show chat")
          .help(showingChat ? "Hide chat (⌘B)" : "Show chat (⌘B)")
      }.customGlassToolbarItem()
    }
    if model.openedSpace != nil {
      ToolbarItem(placement: .navigation) {
        Button {
          model.openedSpace = nil
          showingChat = false
        } label: {
          Image(systemName: "chevron.left").frame(width: 20, height: 20)
        }.nativeControl(circular: true).controlSize(.large)
          .accessibilityLabel("All spaces").help("All spaces")
          .keyboardShortcut("[", modifiers: .command)
          .disabled(model.busy || model.creatingSpace)
      }.customGlassToolbarItem()
    }
    if model.openedSpace != nil {
      ToolbarItem(id: "space-actions", placement: .primaryAction) {
        GlassControls {
          HStack(spacing: 8) {
            if runtime.loading || runtime.catalogLoading {
              ProgressView()
                .controlSize(.small)
                .frame(width: 16, height: 16)
                .accessibilityLabel("Refreshing space")
                .help("Refreshing…")
            }
            Button {
              model.startSpaceChat()
            } label: {
              Image(systemName: "square.and.pencil").frame(width: 20, height: 20)
            }.nativeControl(circular: true).accessibilityLabel("New chat").help("New chat (⌘N)")
              .disabled(chatActionsDisabled)
            Button {
              showHistory = true
            } label: {
              Image(systemName: "clock").frame(width: 20, height: 20)
            }.nativeControl(circular: true).accessibilityLabel("Chat history").help("Chat history")
              .keyboardShortcut("h", modifiers: [.command, .shift])
              .disabled(chatActionsDisabled)
            Divider().frame(height: 18).padding(.horizontal, 4)
            Button {
              sharingSpace = model.openedSpace
            } label: {
              Image(systemName: "square.and.arrow.up").frame(width: 20, height: 20)
            }.nativeControl(circular: true).accessibilityLabel("Share space").help("Share space")
              .disabled(runtime.loading || !runtime.accountConnected)
          }.labelStyle(.titleAndIcon).fixedSize().controlSize(.large)
        }.padding(.vertical, 4)
      }.customGlassToolbarItem()
    }
    if model.openedSpace == nil
      && runtime.showsSpacePicker
    {
      ToolbarItem(id: "glass-space-search", placement: .primaryAction) {
        HStack(spacing: 12) {
          if runtime.loading || runtime.catalogLoading {
            ProgressView()
              .controlSize(.small)
              .frame(width: 16, height: 16)
              .accessibilityLabel("Refreshing spaces")
              .help("Refreshing…")
          }
          GlassToolbarSearch(text: $spaceSearch, focusRequest: searchFocusRequest)
            .frame(width: 240)
        }
      }.customGlassToolbarItem()
    }
    ToolbarItem(id: "more-options", placement: .primaryAction) {
      moreMenu
    }.customGlassToolbarItem()
  }

  private var refreshDisabled: Bool {
    runtime.loading || runtime.catalogLoading || runtime.signInPending
  }

  private func refreshContent() {
    if model.openedSpace == nil { Task { await runtime.refreshSpaces() } } else { runtime.load() }
  }

  private func openSpaceInBrowser() {
    guard let space = model.openedSpace else { return }
    NSWorkspace.shared.open(space.url)
  }

  private func toggleAgentSidebar() {
    showingChat.toggle()
  }

  private var moreMenu: some View {
    AnchoredMenuButton {
      let menu = NSMenu()
      menu.autoenablesItems = false
      menu.addAction(
        "Refresh", symbol: "arrow.clockwise", key: "r",
        enabled: !refreshDisabled, action: refreshContent)
      if model.openedSpace != nil {
        menu.addAction(
          "Open in browser", symbol: "arrow.up.right.square", key: "\u{F703}",
          action: openSpaceInBrowser)
      }
      if let space = model.openedSpace {
        menu.addAction("Sync status…", symbol: "arrow.triangle.2.circlepath") {
          syncStatusSpace = space
        }
      }
      menu.addItem(.separator())
      menu.addAction(
        "Settings…", symbol: "gearshape",
        enabled: !accountActionsDisabled
      ) { showProviderSettings = true }
      if model.provider == .chatGPT && model.signedIn {
        menu.addAction("Sign out of ChatGPT", enabled: !accountActionsDisabled) {
          Task { await model.signOut() }
        }
      }
      if runtime.accountConnected && RuntimeLocation.deployment != .local {
        menu.addAction("Sign out of Tonk", enabled: !accountActionsDisabled) {
          signingOut = true
          Task {
            defer { signingOut = false }
            do {
              try await runtime.signOut()
              model.openedSpace = nil
              showingChat = false
            } catch { accountError = error.localizedDescription }
          }
        }
      }
      return menu
    }
  }

  private var chat: some View {
    VStack(spacing: 0) {
      ScrollViewReader { reader in
        ScrollView {
          LazyVStack(alignment: .leading, spacing: 22) {
            ForEach(ChatTranscript.grouped(model.saved.conversation.messages)) { message in
              VStack(alignment: .leading, spacing: 7) {
                if message.role == "user" {
                  Text("You")
                    .font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                }
                MessageMarkdown(text: message.text)
                  .frame(maxWidth: .infinity, alignment: .leading)
              }
              .padding(message.role == "user" ? 14 : 0)
              .background(
                message.role == "user" ? Color(nsColor: .controlBackgroundColor) : .clear,
                in: RoundedRectangle(cornerRadius: 14)
              )
              .overlay {
                if message.role == "user" {
                  RoundedRectangle(cornerRadius: 14)
                    .strokeBorder(Color(nsColor: .separatorColor), lineWidth: 1)
                    .allowsHitTesting(false)
                }
              }
            }
            if !model.toolActivity.isEmpty {
              DisclosureGroup("Space activity (\(model.toolActivity.count))") {
                ForEach(Array(model.toolActivity.enumerated()), id: \.offset) { _, entry in
                  Text(entry).font(.caption).foregroundStyle(.secondary)
                }
              }.font(.caption)
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
          }.frame(maxWidth: 680).padding(24).frame(maxWidth: .infinity)
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
            Text("Send a message to start")
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
    .background(Color(nsColor: .textBackgroundColor))
    .safeAreaInset(edge: .top, spacing: 0) {
      chatHeader.padding(12).background(Color(nsColor: .textBackgroundColor))
    }
    .safeAreaInset(edge: .bottom, spacing: 0) { composer }
  }

  private var chatHeader: some View {
    GlassControls {
      HStack(spacing: 12) {
        Label("Chat", systemImage: "bubble.left.and.bubble.right").font(.headline)
        Spacer(minLength: 8)
        Button {
          showProviderSettings = true
        } label: {
          HStack(spacing: 6) {
            Text(model.connecting ? "Connecting…" : model.modelLabel).lineLimit(1)
            Image(systemName: "chevron.down").font(.caption2.weight(.semibold))
          }.frame(height: 20)
        }.nativeControl().help("Model settings: \(model.modelLabel)")
          .accessibilityLabel("Model settings: \(model.modelLabel)")
          .disabled(accountActionsDisabled)
        if !model.connected {
          Button("Reconnect", systemImage: "arrow.clockwise") { Task { await model.connect() } }
            .labelStyle(.iconOnly).nativeControl(circular: true)
            .help("Reconnect").disabled(model.connecting)
        }
      }.padding(.horizontal, 8).padding(.vertical, 4)
    }
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
        Button("Configure model") { showProviderSettings = true }.padding(20)
      } else if model.connected && !model.signedIn {
        VStack(spacing: 10) {
          if model.loginPending {
            ProgressView("Continue in your browser")
            Button("Cancel sign-in") { Task { await model.cancelLogin() } }
          } else {
            Button(model.provider == .claude ? "Sign in with Claude" : "Sign in with ChatGPT") {
              Task { await model.signIn() }
            }.nativeControl(
              prominent: true)
            Text(
              model.provider == .claude
                ? "Uses your Claude Code sign-in on this Mac."
                : "Tonk keeps its own sign-in on this Mac."
            ).font(.caption).foregroundStyle(
              .secondary)
          }
        }.frame(maxWidth: .infinity).padding(20)
      } else {
        HStack(alignment: .bottom, spacing: 10) {
          TextField(
            "Message this space…", text: Binding(get: { draft }, set: { draft = $0 }),
            axis: .vertical
          )
          .textFieldStyle(.plain).lineLimit(1...8).focused($composing)
          .frame(minHeight: 28, alignment: .center)
          .onSubmit { submit() }.disabled(!model.canSend)
          .accessibilityLabel("Message")
          if model.busy {
            Button {
              Task { await model.stopTurn() }
            } label: {
              Image(systemName: "stop.fill").frame(width: 20, height: 20)
            }
            .nativeControl(circular: true).disabled(!model.canStop).accessibilityLabel("Stop reply")
          } else {
            Button(action: submit) { Image(systemName: "arrow.up").frame(width: 20, height: 20) }
              .nativeControl(prominent: true, circular: true)
              .disabled(
                !model.canSend || draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
              )
              .accessibilityLabel("Send message")
          }
        }.controlSize(.regular).padding(16).controlSurface(radius: 24)
          .padding(.horizontal, 16).padding(.bottom, 16).padding(.top, 8)
      }
    }
  }

  private var workspace: some View {
    ZStack {
      RuntimeView(model: runtime)
        .allowsHitTesting(runtime.selectedSpace != nil)
        .accessibilityHidden(runtime.selectedSpace == nil)
      if let error = runtime.error {
        ContentUnavailableView {
          Label("Couldn’t open Tonk", systemImage: "network")
        } description: {
          Text(error)
        } actions: {
          Button("Try again") { runtime.load() }
        }.background(.background)
      }
    }
    .background(.background)
    .overlay(alignment: .top) {
      if runtime.loading {
        HStack(spacing: 8) {
          ProgressView().controlSize(.small)
          Text("Loading space…").font(.callout)
        }.padding(.horizontal, 16).padding(.vertical, 10).controlSurface()
          .padding(16).allowsHitTesting(false)
      }
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
