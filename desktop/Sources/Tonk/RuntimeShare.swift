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
    return try Self.validateShareLink(response, subject: space.subject)
  }

  /// An open invitation is a ticket link, `/space/{subject}#{seed}`, or, when
  /// the space kept no ticket, a `/join` link carrying the grant. Either lives
  /// on this deployment or on the host serving the space.
  static func validateShareLink(
    _ response: [String: Any], subject: String, origins: [URL] = [RuntimeLocation.home]
  ) throws -> URL {
    guard response["kind"] as? String == "open",
      let value = response["url"] as? String, let url = URL(string: value),
      url.user == nil, url.password == nil,
      origins.contains(where: {
        url.scheme == $0.scheme && url.host == $0.host && url.port == $0.port
      })
    else { throw HarnessError.message("Tonk returned an unsupported invitation link.") }
    let fragment = url.fragment ?? ""
    let carriesGrant =
      URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems?
      .contains(where: { $0.name == "access" && !($0.value ?? "").isEmpty }) ?? false
    let valid =
      switch url.path {
      case "/space/" + subject: !fragment.isEmpty
      case "/join": !fragment.isEmpty || carriesGrant
      default: false
      }
    guard valid else { throw HarnessError.message("Tonk returned an unsupported invitation link.") }
    return url
  }
}
