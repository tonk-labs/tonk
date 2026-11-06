import Foundation
import HarnessCore
import Security

/// Keys are separate from saved conversations and scoped to the exact API endpoint.
struct ModelCredentials {
  let scope: String
  private func query(_ connection: ModelConnection) -> [String: Any] {
    [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: "xyz.tonk.models.\(scope)",
      kSecAttrAccount as String: connection.provider.rawValue + ":" + connection.baseURL,
    ]
  }
  func read(_ connection: ModelConnection) throws -> String {
    var query = query(connection)
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne
    var value: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &value)
    if status == errSecItemNotFound { return "" }
    guard status == errSecSuccess, let data = value as? Data,
      let key = String(data: data, encoding: .utf8)
    else {
      throw HarnessError.message("Could not read the model API key from Keychain (\(status)).")
    }
    return key
  }
  func write(_ key: String, for connection: ModelConnection) throws {
    let query = query(connection)
    let attributes = [kSecValueData as String: Data(key.utf8)]
    var status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
    if status == errSecItemNotFound {
      var item = query.merging(attributes) { _, new in new }
      item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
      status = SecItemAdd(item as CFDictionary, nil)
    }
    guard status == errSecSuccess else {
      throw HarnessError.message("Could not save the API key in Keychain (\(status)).")
    }
  }
  func remove(_ connection: ModelConnection) throws {
    let status = SecItemDelete(query(connection) as CFDictionary)
    guard status == errSecSuccess || status == errSecItemNotFound else {
      throw HarnessError.message("Could not remove the API key (\(status)).")
    }
  }
}
