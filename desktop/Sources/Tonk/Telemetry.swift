import Foundation
import HarnessCore

/// Sends the desktop's closed analytics events to PostHog, best effort, in small batches.
/// Nothing is sent without a key baked into the release build, when the person turns it
/// off in Settings or with `DO_NOT_TRACK` / `TONK_TELEMETRY=0`, or against a local runtime.
@MainActor
final class Telemetry {
  static let shared = Telemetry()
  /// The Settings choice; absent means on, as for the CLI and web.
  static let enabledKey = "tonkTelemetryEnabled"

  private let apiKey: String?
  private let endpoint: URL
  private let context: DesktopAnalytics.Context
  private let environmentOptsOut: Bool
  private let defaults: UserDefaults
  private var queue: [DesktopAnalytics.Event] = []
  private var distinctID = DesktopAnalytics.anonymousID
  private var flushTask: Task<Void, Never>?

  init(
    environment: [String: String] = ProcessInfo.processInfo.environment, bundle: Bundle = .main,
    defaults: UserDefaults = .standard
  ) {
    let info = { (key: String) in bundle.object(forInfoDictionaryKey: key) as? String }
    apiKey = DesktopAnalytics.apiKey(environment: environment, bundled: info("TonkPostHogKey"))
    endpoint = DesktopAnalytics.host(environment: environment, bundled: info("TonkPostHogHost"))
      .appendingPathComponent("batch/")
    environmentOptsOut = DesktopAnalytics.environmentOptsOut(environment)
    self.defaults = defaults
    #if arch(arm64)
      let arch = "aarch64"
    #else
      let arch = "x86_64"
    #endif
    context = DesktopAnalytics.Context(
      version: info("TonkReleaseVersion") ?? info("CFBundleShortVersionString") ?? "unknown",
      arch: arch)
  }

  /// Whether a build could send at all; Settings hides its toggle otherwise.
  var isAvailable: Bool {
    apiKey != nil && !environmentOptsOut && RuntimeLocation.deployment != .local
  }

  var isEnabled: Bool {
    isAvailable && (defaults.object(forKey: Self.enabledKey) as? Bool ?? true)
  }

  /// Whether a batch is waiting on its timer; for tests.
  var hasScheduledFlush: Bool { flushTask != nil }

  func setEnabled(_ enabled: Bool) {
    defaults.set(enabled, forKey: Self.enabledKey)
    if !enabled {
      queue.removeAll()
      flushTask?.cancel()
      // A cancelled timer left here would stop the next one being scheduled.
      flushTask = nil
    }
  }

  /// The hashed profile, or anonymous before one is known or after sign-out.
  func identify(profileDID: String?) {
    distinctID =
      profileDID.map { DesktopAnalytics.distinctID(profileDID: $0) }
      ?? DesktopAnalytics.anonymousID
  }

  func appOpened() { enqueue(DesktopAnalytics.appOpened(context: context)) }

  /// Starts one product interaction; finish it at its actual receipt.
  func begin(
    _ journey: DesktopAnalytics.Journey, _ action: DesktopAnalytics.Action,
    surface: DesktopAnalytics.Surface, trigger: DesktopAnalytics.Trigger = .user
  ) -> TelemetryAttempt {
    let attempt = TelemetryAttempt(
      journey: journey, action: action, surface: surface, trigger: trigger)
    enqueue(
      DesktopAnalytics.started(
        journey, action, stage: .intent, surface: surface, trigger: trigger,
        attemptID: attempt.id, context: context))
    return attempt
  }

  /// An interaction that completes as it starts, such as opening a window.
  func record(
    _ journey: DesktopAnalytics.Journey, _ action: DesktopAnalytics.Action,
    surface: DesktopAnalytics.Surface, trigger: DesktopAnalytics.Trigger = .user
  ) {
    begin(journey, action, surface: surface, trigger: trigger).finish(.success)
  }

  fileprivate func finish(
    _ attempt: TelemetryAttempt, result: DesktopAnalytics.Result,
    stage: DesktopAnalytics.Stage, failureKind: DesktopAnalytics.FailureKind?
  ) {
    let elapsed = Date().timeIntervalSince(attempt.started)
    enqueue(
      DesktopAnalytics.finished(
        attempt.journey, attempt.action, stage: stage, surface: attempt.surface,
        trigger: attempt.trigger, attemptID: attempt.id, durationMs: Int(elapsed * 1000),
        result: result, failureKind: failureKind, context: context))
  }

  private func enqueue(_ event: DesktopAnalytics.Event) {
    guard isEnabled else { return }
    queue.append(event)
    if queue.count >= 20 {
      flush()
    } else if flushTask == nil {
      // Coalesce a burst of events into one request.
      flushTask = Task { [weak self] in
        try? await Task.sleep(for: .seconds(10))
        guard !Task.isCancelled else { return }
        self?.flush()
      }
    }
  }

  /// Sends what is queued. Failures are dropped: telemetry never retries.
  func flush() {
    guard let request = takeBatch() else { return }
    URLSession.shared.dataTask(with: request).resume()
  }

  /// At quit, waits up to a second for the last batch so it is not lost with the process.
  func flushBeforeExit() {
    guard let request = takeBatch() else { return }
    let done = DispatchSemaphore(value: 0)
    URLSession.shared.dataTask(with: request) { _, _, _ in done.signal() }.resume()
    _ = done.wait(timeout: .now() + 1)
  }

  /// Empties the queue into one `/batch` request.
  private func takeBatch() -> URLRequest? {
    flushTask?.cancel()
    flushTask = nil
    guard isEnabled, let apiKey, !queue.isEmpty else { return nil }
    let body = DesktopAnalytics.batch(queue, apiKey: apiKey, distinctID: distinctID)
    queue.removeAll()
    guard let data = try? JSONEncoder().encode(body) else { return nil }
    var request = URLRequest(url: endpoint, timeoutInterval: 5)
    request.httpMethod = "POST"
    request.setValue("application/json", forHTTPHeaderField: "Content-Type")
    request.httpBody = data
    return request
  }
}

/// One interaction's correlation token and start time.
@MainActor
final class TelemetryAttempt {
  let id = DesktopAnalytics.attemptID()
  let started = Date()
  let journey: DesktopAnalytics.Journey
  let action: DesktopAnalytics.Action
  let surface: DesktopAnalytics.Surface
  let trigger: DesktopAnalytics.Trigger
  private var finished = false

  init(
    journey: DesktopAnalytics.Journey, action: DesktopAnalytics.Action,
    surface: DesktopAnalytics.Surface, trigger: DesktopAnalytics.Trigger
  ) {
    self.journey = journey
    self.action = action
    self.surface = surface
    self.trigger = trigger
  }

  /// The single terminal receipt; later calls are ignored.
  func finish(
    _ result: DesktopAnalytics.Result, stage: DesktopAnalytics.Stage = .complete,
    failureKind: DesktopAnalytics.FailureKind? = nil
  ) {
    guard !finished else { return }
    finished = true
    Telemetry.shared.finish(self, result: result, stage: stage, failureKind: failureKind)
  }
}
