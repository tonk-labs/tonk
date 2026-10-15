import Foundation

/// Transport validation only. The Tonk worker verifies the grant's authority.
public struct TonkAuthorization {
  public let delegation: String
  public let credential: String
  public let remote: String

  public init(data: Data, deployment: RuntimeLocation.Deployment = RuntimeLocation.deployment)
    throws
  {
    guard let grant = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
      throw CallbackError("The browser authorization is not a JSON object.")
    }
    guard let delegation = grant["delegationHex"] as? String, !delegation.isEmpty,
      delegation.count <= 200000, delegation.count.isMultiple(of: 2),
      delegation.utf8.allSatisfy({
        (48...57).contains($0) || (65...70).contains($0) || (97...102).contains($0)
      })
    else {
      throw CallbackError("The browser authorization has a missing or invalid delegationHex field.")
    }
    guard let credential = grant["credentialId"] as? String, !credential.isEmpty else {
      throw CallbackError("The browser authorization has no credentialId field.")
    }
    guard let remote = grant["remote"] as? String,
      let components = URLComponents(string: remote), let url = components.url,
      RuntimeLocation.isEmbedded(url, deployment: deployment),
      ["/ucan", "/ucan/"].contains(components.percentEncodedPath),
      components.query == nil, components.fragment == nil
    else {
      throw CallbackError(
        "The browser authorization must name the selected Tonk deployment’s /ucan/ service.")
    }
    self.delegation = delegation
    self.credential = credential
    self.remote = remote
  }
}
