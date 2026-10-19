import Foundation
import HarnessCore
import Security

/// Bearer link URLs this Mac minted, kept so Copy works later. The worker never
/// stores them; Keychain keeps them off disk and out of the access journal.
protocol AccessLinkStore {
  func read(_ target: String) -> String?
  func write(_ link: String, for target: String) throws
  func remove(_ target: String)
}

struct LinkVault: AccessLinkStore {
  let scope: String

  private func query(_ target: String) -> [String: Any] {
    [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: "xyz.tonk.links.\(scope)",
      kSecAttrAccount as String: target,
    ]
  }

  func read(_ target: String) -> String? {
    var query = query(target)
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne
    var value: CFTypeRef?
    guard SecItemCopyMatching(query as CFDictionary, &value) == errSecSuccess,
      let data = value as? Data
    else { return nil }
    return String(data: data, encoding: .utf8)
  }

  func write(_ link: String, for target: String) throws {
    let query = query(target)
    let attributes = [kSecValueData as String: Data(link.utf8)]
    var status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
    if status == errSecItemNotFound {
      var item = query.merging(attributes) { _, new in new }
      item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
      status = SecItemAdd(item as CFDictionary, nil)
    }
    guard status == errSecSuccess else {
      throw HarnessError.message("Could not keep the link in Keychain (\(status)).")
    }
  }

  func remove(_ target: String) {
    SecItemDelete(query(target) as CFDictionary)
  }
}
