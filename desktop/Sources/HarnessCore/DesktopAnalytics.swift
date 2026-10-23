import CryptoKit
import Foundation

/// The desktop app's analytics, in the closed shapes of `docs/telemetry.md` and
/// `docs/analytics-contract.md`. Nothing here accepts free text: every property comes
/// from these enums or fixed transport context, mirroring `tonk-analytics`.
public enum DesktopAnalytics {
  public static let defaultHost = URL(string: "https://eu.i.posthog.com")!
  /// Before a profile is known, as in the CLI.
  public static let anonymousID = "tonk:anonymous"
  static let schemaVersion = 1
  static let maximumDuration = 600_000

  /// `tonk:<sha256-hex>` of the profile DID: the id the CLI and web use, so one
  /// profile's desktop and web events correlate. The DID never leaves the Mac.
  public static func distinctID(profileDID did: String) -> String {
    "tonk:" + SHA256.hash(data: Data(did.utf8)).map { String(format: "%02x", $0) }.joined()
  }

  /// The CLI's rule: `DO_NOT_TRACK` set to anything non-empty except `0`, or
  /// `TONK_TELEMETRY` set to `0`, `false` or `off`.
  public static func environmentOptsOut(_ environment: [String: String]) -> Bool {
    if let value = environment["DO_NOT_TRACK"], !value.isEmpty, value != "0" { return true }
    return ["0", "false", "off"].contains(environment["TONK_TELEMETRY"] ?? "")
  }

  /// The project key: a runtime `TONK_POSTHOG_KEY` first, then the key baked into the
  /// release build. `nil` sends nothing, the default for local builds and forks.
  public static func apiKey(environment: [String: String], bundled: String?) -> String? {
    for value in [environment["TONK_POSTHOG_KEY"], bundled] {
      if let value, !value.isEmpty { return value }
    }
    return nil
  }

  /// `TONK_POSTHOG_ENDPOINT` (tests) or `TONK_POSTHOG_HOST`, then the bundled host,
  /// then PostHog's EU cloud.
  public static func host(environment: [String: String], bundled: String?) -> URL {
    for value in [environment["TONK_POSTHOG_ENDPOINT"], environment["TONK_POSTHOG_HOST"], bundled] {
      if let value, !value.isEmpty, let url = URL(string: value), url.scheme != nil {
        return url
      }
    }
    return defaultHost
  }

  /// An opaque per-attempt token, never an account or content identifier.
  public static func attemptID() -> String {
    (0..<16).map { _ in String(format: "%02x", UInt8.random(in: .min ... .max)) }.joined()
  }
}

// MARK: - Closed vocabulary (snake_case on the wire, as in `tonk_analytics::product`)

extension DesktopAnalytics {
  public enum Journey: String, Sendable {
    case startup, account, space, collaboration, workspace, handoff
  }

  /// Desktop-owned operations; each name also exists in `ProductAction`.
  public enum Action: String, Sendable, CaseIterable {
    case copyShareLink = "copy_share_link"
    case inviteContact = "invite_contact"
    case connectAgent = "connect_agent"
    case publishSpace = "publish_space"
    case unpublishSpace = "unpublish_space"
    case revokeLink = "revoke_link"
    case resetLink = "reset_link"
    case openView = "open_view"
    case startChat = "start_chat"
    case sendMessage = "send_message"
    case changeModel = "change_model"
  }

  public enum Stage: String, Sendable {
    case intent, validation, worker, clipboard, ready, complete
    case localCommit = "local_commit"
    case remoteCommit = "remote_commit"
  }

  public enum Surface: String, Sendable { case hub, workspace, settings }
  public enum Trigger: String, Sendable { case user, automatic, recovery }

  public enum Result: String, Sendable {
    case success, cancelled, blocked, noop
    case degradedSuccess = "degraded_success"
    case retryableFailure = "retryable_failure"
    case terminalFailure = "terminal_failure"
    case unknownCommit = "unknown_commit"

    var isSuccess: Bool { self == .success || self == .degradedSuccess || self == .noop }
  }

  public enum FailureKind: String, Sendable {
    case cancelled, timeout, unsupported, conflict, network, unknown
    case invalidInput = "invalid_input"
    case accessDenied = "access_denied"
    case notFound = "not_found"
    case serviceUnavailable = "service_unavailable"
    case invalidResponse = "invalid_response"
    case localState = "local_state"
  }
}

// MARK: - Events

extension DesktopAnalytics {
  /// One queued item, before identity is attached at send time.
  public struct Event: Sendable, Equatable {
    public let name: String
    public let properties: [String: JSONValue]
    /// RFC 3339 UTC, recorded when queued so batching keeps the real order.
    public let timestamp: String
  }

  /// Fixed transport context added to every event.
  public struct Context: Sendable, Equatable {
    public var version: String
    public var arch: String
    public init(version: String, arch: String) {
      self.version = version
      self.arch = arch
    }
  }

  static func timestamp(_ date: Date) -> String {
    let format = ISO8601DateFormatter()
    format.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return format.string(from: date)
  }

  static func event(
    _ name: String, _ properties: [String: JSONValue], context: Context, at date: Date
  ) -> Event {
    var properties = properties
    properties["$lib"] = .string("tonk-desktop")
    properties["environment"] = .string("desktop")
    properties["version"] = .string(context.version)
    properties["os"] = .string("macos")
    properties["arch"] = .string(context.arch)
    return Event(name: name, properties: properties, timestamp: timestamp(date))
  }

  /// `app_opened`: transport context only.
  public static func appOpened(context: Context, at date: Date = Date()) -> Event {
    event("app_opened", [:], context: context, at: date)
  }

  public static func started(
    _ journey: Journey, _ action: Action, stage: Stage, surface: Surface, trigger: Trigger,
    attemptID: String, context: Context, at date: Date = Date()
  ) -> Event {
    product(
      journey, action, phase: "started", stage: stage, surface: surface, trigger: trigger,
      attemptID: attemptID, context: context, at: date)
  }

  /// The terminal receipt. A failure kind travels only with a non-success result, and the
  /// duration is capped at ten minutes, as the contract validates.
  public static func finished(
    _ journey: Journey, _ action: Action, stage: Stage, surface: Surface, trigger: Trigger,
    attemptID: String, durationMs: Int, result: Result, failureKind: FailureKind? = nil,
    context: Context, at date: Date = Date()
  ) -> Event {
    var terminal: [String: JSONValue] = [
      "result": .string(result.rawValue),
      "duration_ms": .number(Double(min(max(durationMs, 0), maximumDuration))),
    ]
    if !result.isSuccess {
      terminal["failure_kind"] = .string((failureKind ?? .unknown).rawValue)
    }
    return product(
      journey, action, phase: "finished", stage: stage, surface: surface, trigger: trigger,
      attemptID: attemptID, context: context, at: date, terminal: terminal)
  }

  static func product(
    _ journey: Journey, _ action: Action, phase: String, stage: Stage, surface: Surface,
    trigger: Trigger, attemptID: String, context: Context, at date: Date,
    terminal: [String: JSONValue] = [:]
  ) -> Event {
    var properties: [String: JSONValue] = [
      "schema_version": .number(Double(schemaVersion)),
      "journey": .string(journey.rawValue),
      "action": .string(action.rawValue),
      "phase": .string(phase),
      "stage": .string(stage.rawValue),
      "surface": .string(surface.rawValue),
      "trigger": .string(trigger.rawValue),
      "attempt_id": .string(attemptID),
    ]
    properties.merge(terminal) { _, new in new }
    return event("product_event", properties, context: context, at: date)
  }

  /// The `/batch` request body. Identity is attached here, so events queued before the
  /// profile resolved still carry it.
  public static func batch(_ events: [Event], apiKey: String, distinctID: String) -> JSONValue {
    .object([
      "api_key": .string(apiKey),
      "batch": .array(
        events.map { event in
          .object([
            "event": .string(event.name),
            "distinct_id": .string(distinctID),
            "timestamp": .string(event.timestamp),
            "properties": .object(event.properties),
          ])
        }),
    ])
  }
}
