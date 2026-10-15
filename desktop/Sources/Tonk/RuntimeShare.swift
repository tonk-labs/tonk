import Foundation
import HarnessCore

@MainActor
extension RuntimeModel {
  func createShareLink(_ space: TonkSpace) async throws -> URL {
    try requireSpaceReady(space)
    let response = try await accountScript(
      """
      return await api('/api/repository/' + encodeURIComponent(subject) + '/invite', {});
      """, arguments: ["subject": space.subject])
    return try Self.validateShareLink(response, origin: RuntimeLocation.home)
  }

  static func validateShareLink(_ response: [String: Any], origin: URL) throws -> URL {
    guard response["kind"] as? String == "open",
      let value = response["url"] as? String, let url = URL(string: value),
      url.scheme == origin.scheme, url.host == origin.host, url.port == origin.port,
      url.user == nil, url.password == nil, url.path == "/join",
      let fragment = url.fragment, !fragment.isEmpty
    else { throw HarnessError.message("Tonk returned an unsupported invitation link.") }
    return url
  }
}
