import Foundation
import HarnessCore

@MainActor
extension RuntimeModel {
  /// Pull only the attached branch; leave navigation, focus and input to the live renderer.
  func synchronizeAfterCLIWrite(_ space: TonkSpace) async throws {
    try requireSpaceReady(space)
    try Task.checkCancellation()
    _ = try await accountScript(Self.pullSpaceScript, arguments: ["subject": space.subject])
    try Task.checkCancellation()
    try requireSpaceReady(space)
  }

  static let pullSpaceScript = """
    const result = await api('/api/repository/' + encodeURIComponent(subject) + '/branch/main/sync/pull', {});
    if (result.success !== true)
      throw new Error('The space update could not be synchronized.');
    return {pulled: true};
    """
}
