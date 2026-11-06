import AppKit
import HarnessCore
import SwiftUI

@main
struct TonkApp: App {
  @NSApplicationDelegateAdaptor(AppDelegate.self) var delegate
  /// The one saved chat state, shared by the hub and every space window.
  private let archive: ChatArchive
  /// The hub's chat model: provider settings and sign-in. Space windows have their own.
  @StateObject private var model: HarnessModel
  @StateObject private var runtime = RuntimeModel()
  @StateObject private var access = AccessModel()

  init() {
    let archive = ChatArchive(directory: HarnessModel.defaultRoot)
    self.archive = archive
    _model = StateObject(wrappedValue: HarnessModel(store: archive.handle(claimsChats: false)))
  }

  var body: some Scene {
    Window(RuntimeLocation.deployment.title, id: "main") {
      ContentView(model: model, runtime: runtime, access: access)
        .frame(minWidth: 700, minHeight: 500)
        .background(WindowAppearance())
        .modifier(OpenActivityFixture())
        .task {
          if ActivityFixture.requested { return }
          if ProcessInfo.processInfo.arguments.contains("--links-fixture") {
            ActivityFixture.prepareLinks(runtime: runtime, access: access)
            ActivityFixture.snapshotIfRequested(runtime: runtime, access: access)
            return
          }
          runtime.prepareCatalogCache(directory: model.root)
          access.prepare(
            directory: model.root, runtime: runtime, expectedRoot: runtime.cachedAccountRoot)
          delegate.archive = archive
          delegate.runtime = runtime
          model.runtime = runtime
          await runtime.startMCPBridge(root: model.root)
          runtime.load()
          if ProcessInfo.processInfo.arguments.contains("--agent-workflow-test") {
            await AgentWorkflowSmoke.run(model: model, runtime: runtime)
          } else if ProcessInfo.processInfo.arguments.contains("--diagnose-sync") {
            await SmokeTest.diagnoseSync(runtime: runtime)
          } else if ProcessInfo.processInfo.arguments.contains("--smoke-test") {
            await model.connect()
            await SmokeTest.run(model: model, runtime: runtime)
          }
        }
    }
    .defaultSize(width: 800, height: 600)
    .windowToolbarStyle(.unified)
    .commands {
      WindowCommands()
    }

    // One window per space view, each with its own page and chat.
    WindowGroup("Space", for: SpaceWindowRequest.self) { $request in
      if let request {
        SpaceWindow(request: request, runtime: runtime, access: access, archive: archive)
          .frame(minWidth: 700, minHeight: 500)
          .background(WindowAppearance())
      }
    }
    .defaultSize(
      width: ActivityFixture.compactWindow ? 700 : 1200,
      height: ActivityFixture.compactWindow ? 500 : 800
    )
    .windowStyle(.hiddenTitleBar)
    .windowToolbarStyle(.unified)
    // A restored window would open before the account and its spaces load.
    .restorationBehavior(.disabled)
  }
}

/// The activity fixture draws a space window with a chat, so it opens one.
private struct OpenActivityFixture: ViewModifier {
  @Environment(\.openWindow) private var openWindow

  func body(content: Content) -> some View {
    content.task {
      guard ActivityFixture.requested, let space = ActivityFixture.space else { return }
      openWindow(value: SpaceWindowRequest(subject: space.subject, url: nil))
      if ProcessInfo.processInfo.arguments.contains("--multiple-windows-fixture") {
        openWindow(value: SpaceWindowRequest(subject: space.subject, url: space.url))
      }
    }
  }
}

private struct WindowAppearance: NSViewRepresentable {
  func makeNSView(context: Context) -> NSView { SeparatorlessView() }
  func updateNSView(_ nsView: NSView, context: Context) {}

  private final class SeparatorlessView: NSView {
    override func viewDidMoveToWindow() {
      super.viewDidMoveToWindow()
      // Let the window material blend into the glass control layer.
      window?.titlebarSeparatorStyle = .none
    }
  }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
  var archive: ChatArchive?
  weak var runtime: RuntimeModel?
  func applicationDidFinishLaunching(_ notification: Notification) {
    NSApp.setActivationPolicy(.regular)
    Telemetry.shared.appOpened()
    AppUpdater.shared.start()
    // Resolve the compiled bundle icon explicitly: automatic AppKit lookup can
    // leave this SwiftPM-built app with a transparent Dock and switcher icon.
    if Bundle.main.object(forInfoDictionaryKey: "CFBundleIconName") != nil {
      let icon = NSWorkspace.shared.icon(forFile: Bundle.main.bundlePath)
      let size = NSSize(width: 1024, height: 1024)
      icon.size = size
      // Supply actual pixels, not just a logical point size, to the Dock.
      if let bitmap = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: 1024, pixelsHigh: 1024,
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0),
        let context = NSGraphicsContext(bitmapImageRep: bitmap)
      {
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = context
        icon.draw(in: NSRect(origin: .zero, size: size))
        NSGraphicsContext.restoreGraphicsState()
        let dockIcon = NSImage(size: size)
        dockIcon.addRepresentation(bitmap)
        NSApp.applicationIconImage = dockIcon
      }
    }
    NSApp.activate(ignoringOtherApps: true)
  }
  func applicationWillTerminate(_ notification: Notification) {
    Telemetry.shared.flushBeforeExit()
    runtime?.stopMCPBridge()
    // Saves and stops the hub's chat model and every space window's.
    archive?.shutdown()
  }
  func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}
