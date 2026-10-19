import AppKit
import HarnessCore
import SwiftUI

@main
struct TonkApp: App {
  @NSApplicationDelegateAdaptor(AppDelegate.self) var delegate
  @StateObject private var model = HarnessModel()
  @StateObject private var runtime = RuntimeModel()
  @StateObject private var access = AccessModel()

  var body: some Scene {
    Window(RuntimeLocation.deployment.title, id: "main") {
      ContentView(model: model, runtime: runtime, access: access)
        .frame(minWidth: 850, minHeight: 580)
        .background(WindowAppearance())
        .task {
          if ProcessInfo.processInfo.arguments.contains("--activity-fixture") {
            ActivityFixture.prepare(model: model, runtime: runtime)
            return
          }
          if ProcessInfo.processInfo.arguments.contains("--links-fixture") {
            ActivityFixture.prepareLinks(runtime: runtime, access: access)
            ActivityFixture.snapshotIfRequested(runtime: runtime, access: access)
            return
          }
          runtime.prepareCatalogCache(directory: model.root)
          access.prepare(directory: model.root, runtime: runtime)
          delegate.model = model
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
    .defaultSize(width: 1180, height: 780)
    .windowToolbarStyle(.unified)
    .commands {
      CommandGroup(replacing: .newItem) {
        Button("New Chat") {
          model.startSpaceChat()
        }
        .keyboardShortcut("n", modifiers: [.command, .shift]).disabled(
          model.openedSpace == nil || model.busy || model.creatingSpace || model.loginPending
            || model.connecting)
      }
      WindowCommands()
    }

    // One space at a time, with its chat; opening another view reuses it.
    Window("Space", id: "space") {
      ContentView(model: model, runtime: runtime, access: access, isSpace: true)
        .frame(minWidth: 700, minHeight: 500)
        .background(WindowAppearance())
    }
    .defaultSize(width: 1180, height: 780)
    .windowToolbarStyle(.unified)
    // A restored window would have no space to show.
    .restorationBehavior(.disabled)
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
  weak var model: HarnessModel?
  weak var runtime: RuntimeModel?
  func applicationDidFinishLaunching(_ notification: Notification) {
    NSApp.setActivationPolicy(.regular)
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
    runtime?.stopMCPBridge()
    model?.shutdown()
  }
  func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}
