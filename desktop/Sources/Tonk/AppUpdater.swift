import Combine
import Sparkle

/// Sparkle updates for published builds. Only a bundle stamped with a feed and
/// its EdDSA public key starts an updater, so local, ad-hoc and test copies
/// never replace themselves. Sparkle asks before installing and relaunching.
@MainActor
final class AppUpdater: ObservableObject {
  static let shared = AppUpdater()

  @Published private(set) var canCheckForUpdates = false
  private var controller: SPUStandardUpdaterController?

  /// Disposable test copies run with their own data or a local runtime.
  private nonisolated static let testArguments: Set<String> = ["--data-dir", "--local-runtime"]

  nonisolated static func isEnabled(info: [String: Any], arguments: [String]) -> Bool {
    func present(_ key: String) -> Bool {
      !((info[key] as? String)?.trimmingCharacters(in: .whitespaces).isEmpty ?? true)
    }
    return present("SUFeedURL") && present("SUPublicEDKey")
      && testArguments.isDisjoint(with: arguments)
  }

  /// Starts once the app has launched; Sparkle schedules its own background checks.
  func start() {
    guard controller == nil,
      Self.isEnabled(
        info: Bundle.main.infoDictionary ?? [:], arguments: ProcessInfo.processInfo.arguments)
    else { return }
    let controller = SPUStandardUpdaterController(
      startingUpdater: true, updaterDelegate: nil, userDriverDelegate: nil)
    self.controller = controller
    controller.updater.publisher(for: \.canCheckForUpdates).assign(to: &$canCheckForUpdates)
  }

  var isAvailable: Bool { controller != nil }

  func checkForUpdates() { controller?.checkForUpdates(nil) }
}
