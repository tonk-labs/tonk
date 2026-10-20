import HarnessCore
import SwiftUI

@MainActor
final class DesktopSyncState: ObservableObject {
  @Published var status = "Checking sync…"
  @Published var detail = "Comparing this space with its remote."
  @Published var state = "unknown"
  @Published var checking = false
  @Published var changing = false
  @Published var checkedAt: Date?
  var symbol: String {
    switch state {
    case "synced", "sync:idle": "circle.fill"
    case "sync:paused": "circle.lefthalf.filled"
    case "ahead", "behind", "sync:pending": "arrow.triangle.2.circlepath"
    case "diverged": "exclamationmark.circle.fill"
    default: "circle"
    }
  }
  var canToggle: Bool {
    !["unknown", "error", "sync:offline", "no-upstream", "sync:local"].contains(state) && !changing
  }

  static let script = """
    const base = '/api/repository/' + encodeURIComponent(subject) + '/branch/main';
    const probe = await api(base + '/sync/status');
    const rows = await api(base + '/query', {
      predicate: {with: {status: {the: 'xyz.tonk.sync/status', as: 'Entity', cardinality: 'one'}}},
      terms: {this: 'state:here', status: {'?': {name: 'status'}}}
    });
    const observed = rows[0]?.fields?.status;
    return {state: ['sync:paused', 'sync:offline', 'sync:pending'].includes(observed) ? observed : probe.state};
    """

  func check(runtime: RuntimeModel, space: TonkSpace) async {
    guard !checking, !changing else { return }
    checking = true
    defer { checking = false }
    do {
      let result = try await runtime.accountScript(
        Self.script, arguments: ["subject": space.subject])
      guard !Task.isCancelled else { return }
      state = result["state"] as? String ?? "unknown"
      switch state {
      case "synced", "sync:idle":
        status = "Synced"
        detail = "\(space.title) is up to date on this Mac."
      case "sync:paused":
        status = "Paused"
        detail = "Automatic syncing is paused on this Mac."
      case "sync:offline":
        status = "Offline"
        detail = "This Mac cannot reach the remote right now."
      case "ahead":
        status = "Changes to upload"
        detail = "This Mac has changes to sync."
      case "behind":
        status = "Changes to download"
        detail = "The remote has newer changes."
      case "sync:pending":
        status = "Syncing"
        detail = "Sync is in progress."
      case "diverged":
        status = "Needs reconciliation"
        detail = "Local and remote revisions differ."
      case "no-upstream":
        status = "Not syncing"
        detail = "No upstream is available, or sync is paused."
      default:
        status = "Sync status unavailable"
        detail = "The runtime did not provide a recognised sync state."
      }
      checkedAt = Date()
    } catch {
      guard !Task.isCancelled else { return }
      state = "error"
      status = "Couldn’t check sync"
      detail = AccessModel.message(error)
    }
  }

  func toggle(runtime: RuntimeModel, space: TonkSpace) async {
    guard canToggle, let branch = runtime.catalogBranch else { return }
    changing = true
    do {
      _ = try await runtime.accountScript(
        """
        const field = (the, as) => ({the, as, cardinality: 'one'});
        await api('/api/profile/branch/' + encodeURIComponent(branch) + '/transact', {claims: [{op: 'assert', application: {
          predicate: {kind: 'transient', concept: {with: {
            space: field('xyz.tonk.pause-sync/space', 'Entity'),
            time: field('xyz.tonk.command.pause-sync/time', 'Float')
          }}}, parameters: {space: subject, time: Date.now()}
        }}]});
        return {submitted: true};
        """, arguments: ["subject": space.subject, "branch": branch])
      changing = false
      await check(runtime: runtime, space: space)
    } catch {
      changing = false
      detail = "Couldn’t change syncing: " + AccessModel.message(error)
    }
  }
}

struct SyncStatusView: View {
  @ObservedObject var runtime: RuntimeModel
  let space: TonkSpace
  @StateObject private var observation: DesktopSyncState
  init(runtime: RuntimeModel, space: TonkSpace, observation: DesktopSyncState? = nil) {
    self.runtime = runtime
    self.space = space
    _observation = StateObject(wrappedValue: observation ?? DesktopSyncState())
  }
  var body: some View {
    VStack(alignment: .leading, spacing: 10) {
      Label(observation.status, systemImage: observation.symbol).fontWeight(.medium)
      Text(observation.detail).font(.system(size: 12)).foregroundStyle(.secondary).textSelection(
        .enabled)
      HStack {
        if observation.canToggle {
          Button(observation.state == "sync:paused" ? "Resume Syncing" : "Pause Syncing") {
            Task { await observation.toggle(runtime: runtime, space: space) }
          }
        }
        Button("Check Again") { Task { await observation.check(runtime: runtime, space: space) } }
          .disabled(observation.checking)
      }
      if observation.canToggle {
        Text("Or hold the sync circle for half a second to pause or resume.")
          .font(.system(size: 12)).foregroundStyle(.secondary)
      }
      if let date = observation.checkedAt {
        Text("Checked \(date.formatted(date: .omitted, time: .shortened))")
          .font(.system(size: 12)).foregroundStyle(.tertiary)
      }
    }.font(.system(size: 13)).padding(16).frame(width: 270)
      .task { await observation.check(runtime: runtime, space: space) }
  }
}
