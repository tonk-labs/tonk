import SwiftUI

private struct ModelSettingsKey: FocusedValueKey { typealias Value = () -> Void }
private struct RefreshContentKey: FocusedValueKey { typealias Value = () -> Void }
private struct SearchFocusKey: FocusedValueKey { typealias Value = () -> Void }
private struct OpenSpaceInBrowserKey: FocusedValueKey { typealias Value = () -> Void }
private struct ToggleAgentSidebarKey: FocusedValueKey { typealias Value = () -> Void }
private struct NewLinkKey: FocusedValueKey { typealias Value = () -> Void }
private struct NewChatKey: FocusedValueKey { typealias Value = () -> Void }

extension FocusedValues {
  var newChat: (() -> Void)? {
    get { self[NewChatKey.self] }
    set { self[NewChatKey.self] = newValue }
  }
  var newLink: (() -> Void)? {
    get { self[NewLinkKey.self] }
    set { self[NewLinkKey.self] = newValue }
  }
  var toggleAgentSidebar: (() -> Void)? {
    get { self[ToggleAgentSidebarKey.self] }
    set { self[ToggleAgentSidebarKey.self] = newValue }
  }
  var openSpaceInBrowser: (() -> Void)? {
    get { self[OpenSpaceInBrowserKey.self] }
    set { self[OpenSpaceInBrowserKey.self] = newValue }
  }
  var refreshContent: (() -> Void)? {
    get { self[RefreshContentKey.self] }
    set { self[RefreshContentKey.self] = newValue }
  }
  var modelSettings: (() -> Void)? {
    get { self[ModelSettingsKey.self] }
    set { self[ModelSettingsKey.self] = newValue }
  }
  var focusSearch: (() -> Void)? {
    get { self[SearchFocusKey.self] }
    set { self[SearchFocusKey.self] = newValue }
  }
}

struct WindowCommands: Commands {
  @FocusedValue(\.modelSettings) private var modelSettings
  @FocusedValue(\.focusSearch) private var focusSearch

  @FocusedValue(\.refreshContent) private var refreshContent
  @FocusedValue(\.openSpaceInBrowser) private var openSpaceInBrowser
  @FocusedValue(\.toggleAgentSidebar) private var toggleAgentSidebar
  @FocusedValue(\.newLink) private var newLink
  @FocusedValue(\.newChat) private var newChat

  @ObservedObject private var updater = AppUpdater.shared

  var body: some Commands {
    CommandGroup(after: .appInfo) {
      if updater.isAvailable {
        Button("Check for Updates…") { updater.checkForUpdates() }
          .disabled(!updater.canCheckForUpdates)
      }
    }
    CommandGroup(replacing: .appSettings) {
      Button("Settings…") { modelSettings?() }
        .keyboardShortcut(",").disabled(modelSettings == nil)
    }
    CommandGroup(before: .newItem) {
      Button("New Link…") { newLink?() }
        .keyboardShortcut("n")
        .disabled(newLink == nil)
    }
    CommandGroup(replacing: .newItem) {
      // Each space window has its own chat; this starts one in the focused window.
      Button("New Chat") { newChat?() }
        .keyboardShortcut("n", modifiers: [.command, .shift])
        .disabled(newChat == nil)
    }
    CommandGroup(after: .toolbar) {
      Button("Refresh") { refreshContent?() }
        .keyboardShortcut("r").disabled(refreshContent == nil)
      Button("Toggle Agent Sidebar") { toggleAgentSidebar?() }
        .keyboardShortcut("b", modifiers: .command)
        .disabled(toggleAgentSidebar == nil)
      Button("Open in Browser") { openSpaceInBrowser?() }
        .keyboardShortcut(.rightArrow, modifiers: .command)
        .disabled(openSpaceInBrowser == nil)
    }
    CommandGroup(after: .textEditing) {
      Button("Find…") { focusSearch?() }
        .keyboardShortcut("f").disabled(focusSearch == nil)
    }
  }
}
