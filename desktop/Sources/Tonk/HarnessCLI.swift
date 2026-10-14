import Foundation
import HarnessCore

@MainActor
extension HarnessModel {
  func cliAdapter(for space: TonkSpace) -> TonkCLI {
    if let cli = cliAdapters[space.subject] { return cli }
    let cli = TonkCLI(root: root, subject: space.subject)
    cliAdapters[space.subject] = cli
    return cli
  }

  func prepareCLI(for space: TonkSpace, runtime: RuntimeModel) async throws -> TonkCLI {
    try runtime.requireSpaceReady(space)
    let cli = cliAdapter(for: space)
    if cli.pendingLink != nil || !cli.isConnected {
      activity = "Connecting to space"
      toolActivity.append("Preparing space tools")
    }
    try await cli.ensureConnected { try await runtime.createCLILink(space) }
    try Task.checkCancellation()
    return cli
  }
}
