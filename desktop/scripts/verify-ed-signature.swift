// Check an EdDSA signature against the public key a build ships with, so a
// mismatched key pair fails the release instead of every installed update.
// Usage: swift verify-ed-signature.swift PUBLIC_KEY_BASE64 SIGNATURE_BASE64 FILE
import CryptoKit
import Foundation

let arguments = CommandLine.arguments.dropFirst()
guard arguments.count == 3,
  let publicKey = Data(base64Encoded: arguments[arguments.startIndex]),
  let signature = Data(base64Encoded: arguments[arguments.startIndex + 1]),
  let file = FileManager.default.contents(atPath: arguments[arguments.startIndex + 2])
else {
  FileHandle.standardError.write(Data("Usage: verify-ed-signature PUBLIC_KEY SIGNATURE FILE\n".utf8))
  exit(2)
}
guard (try? Curve25519.Signing.PublicKey(rawRepresentation: publicKey))?
  .isValidSignature(signature, for: file) == true
else {
  FileHandle.standardError.write(Data("Signature does not match the app's update key.\n".utf8))
  exit(1)
}
