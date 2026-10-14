import HarnessCore
import SwiftUI

struct SyncStatusView: View {
  @ObservedObject var runtime: RuntimeModel
  let space: TonkSpace
  @Environment(\.dismiss) private var dismiss
  @State private var status = "Checking…"
  @State private var detail = "Comparing this space with its remote."
  @State private var checkedAt: Date?
  @State private var checking = false
  @State private var synced = false

  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      Text("Sync status").font(.title2.weight(.semibold))
      Text(space.title).foregroundStyle(.secondary)
      HStack {
        Image(systemName: synced ? "checkmark.circle" : "arrow.triangle.2.circlepath")
          .foregroundStyle(synced ? Color.green : Color.secondary)
        Text(status).font(.headline)
        if checking { ProgressView().controlSize(.small) }
      }
      Text(detail).font(.callout).textSelection(.enabled)
      if let checkedAt {
        Text("Checked \(checkedAt.formatted(date: .omitted, time: .standard)) · main branch")
          .font(.caption).foregroundStyle(.secondary)
      }
      HStack {
        Button("Check again") { Task { await check() } }.disabled(checking)
        Spacer()
        Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
      }
    }
    .padding(24).frame(width: 420).nativeControl()
    .task {
      while !Task.isCancelled {
        await check()
        do { try await Task.sleep(for: .seconds(5)) } catch { return }
      }
    }
  }

  @MainActor private func check() async {
    guard !checking else { return }
    checking = true
    defer { checking = false }
    do {
      let result = try await runtime.accountScript(
        """
        const response = await fetch('/api/repository/' + encodeURIComponent(subject) + '/branch/main/sync/status', {
          signal: AbortSignal.timeout(15000)
        });
        const body = await response.json();
        if (!response.ok) {
          const code = body.code || body.error?.code;
          return {state: 'error', detail: code ? String(code) : 'HTTP ' + response.status};
        }
        return {state: body.state};
        """, arguments: ["subject": space.subject])
      guard !Task.isCancelled else { return }
      synced = result["state"] as? String == "synced"
      switch result["state"] as? String {
      case "synced":
        status = "Up to date"
        detail = "Local and remote revisions match."
      case "ahead":
        status = "Changes to upload"
        detail = "This device has changes the remote does not have yet."
      case "behind":
        status = "Changes to download"
        detail = "The remote has changes this device does not have yet."
      case "diverged":
        status = "Needs reconciliation"
        detail = "Local and remote revisions differ and need to be reconciled."
      case "no-upstream":
        status = "Not syncing"
        detail = "Sync is paused or this branch has no remote."
      default:
        status = "Couldn’t check sync"
        detail = result["detail"] as? String ?? "The runtime returned an unknown sync state."
      }
      checkedAt = Date()
    } catch {
      guard !Task.isCancelled else { return }
      synced = false
      status = "Couldn’t check sync"
      detail = (error as NSError).userInfo["WKJavaScriptExceptionMessage"] as? String
        ?? error.localizedDescription
      checkedAt = Date()
    }
  }
}
