import Foundation
import OSLog
import SwiftUI

struct BrowserSignInProgress {
  enum Stage: String {
    case preparing, browser, authorization, account

    var title: String {
      switch self {
      case .preparing: "Preparing sign-in…"
      case .browser: "Waiting for browser approval…"
      case .authorization: "Authorization received"
      case .account: "Connecting your account…"
      }
    }

    var detail: String {
      switch self {
      case .preparing: "Checking this device before opening your browser."
      case .browser:
        "Complete the passkey prompt, then keep the Tonk tab open while it prepares your connection."
      case .authorization: "Verifying and saving your device authorization."
      case .account: "Restoring your account and preparing your spaces. Keep Tonk open."
      }
    }
  }

  let stage: Stage
  let started = Date()
  let uptime = ProcessInfo.processInfo.systemUptime
}

@MainActor
extension RuntimeModel {
  func setBrowserSignInStage(_ stage: BrowserSignInProgress.Stage?) {
    if let previous = browserSignInProgress {
      let elapsed = Int((ProcessInfo.processInfo.systemUptime - previous.uptime) * 1000)
      // Only fixed stage names and duration: no identifiers, grants, or URLs.
      Logger(subsystem: "xyz.tonk", category: "sign-in").info(
        "browser-sign-in stage=\(previous.stage.rawValue, privacy: .public) elapsed_ms=\(elapsed)")
    }
    browserSignInProgress = stage.map { BrowserSignInProgress(stage: $0) }
  }
}

struct BrowserSignInProgressView: View {
  let progress: BrowserSignInProgress

  var body: some View {
    VStack(spacing: 10) {
      ProgressView(progress.stage.title)
      Text(progress.stage.detail)
        .font(.callout).foregroundStyle(.secondary).multilineTextAlignment(.center)
      TimelineView(.periodic(from: progress.started, by: 1)) { context in
        let seconds = max(0, Int(context.date.timeIntervalSince(progress.started)))
        if seconds >= 10 {
          Text(
            "\(seconds)s elapsed" + (seconds >= 30 ? " · This is taking longer than usual." : "")
          )
          .font(.caption).foregroundStyle(.secondary).monospacedDigit()
        }
      }
    }
  }
}
