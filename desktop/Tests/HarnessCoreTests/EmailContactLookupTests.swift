import Foundation
import XCTest

@testable import HarnessCore

final class EmailContactLookupTests: XCTestCase {
  private let directory = URL(string: "https://tonk.example")!

  func testNormalizesAndEncodesEmailOnce() throws {
    let query = try EmailContactLookup.Query("  Tag+ALICE@Example.COM ", directory: directory)
    XCTAssertEqual(query.email, "tag+alice@example.com")
    XCTAssertEqual(
      query.url.absoluteString, "https://tonk.example/customer/example.com/tag%2Balice/did.json")
    XCTAssertEqual(query.did, "did:web:tonk.example:customer:example.com:tag%2Balice")
    for (input, encoded) in [("a%b", "a%25b"), ("josé", "jos%C3%A9"), ("a?b#c", "a%3Fb%23c")] {
      let query = try EmailContactLookup.Query(input + "@example.com", directory: directory)
      XCTAssertTrue(query.url.absoluteString.contains("/" + encoded + "/did.json"))
    }
    let local = try EmailContactLookup.Query(
      "a@example.com", directory: URL(string: "http://localhost:8090")!)
    XCTAssertTrue(local.did.hasPrefix("did:web:localhost%3A8090:"))
  }

  func testRejectsUnresolvableAddressesAndUnsafeOrigins() {
    for email in [
      "", "alice", "@example.com", "a@", "a@b@example.com", "a/b@example.com", "a b@example.com",
      "a@../x", ".@example.com", "..@example.com", "a@a..b",
    ] {
      XCTAssertThrowsError(try EmailContactLookup.Query(email, directory: directory), email)
    }
    for origin in [
      "http://tonk.example", "https://user@tonk.example", "https://tonk.example/path",
      "https://tonk.example?query",
    ] {
      XCTAssertThrowsError(
        try EmailContactLookup.Query("a@example.com", directory: URL(string: origin)!))
    }
  }

  func testOnlyAnActiveConsistentDocumentYieldsAKey() throws {
    let query = try EmailContactLookup.Query("alice@example.com", directory: directory)
    let key = "z6MkvWgxPPHxXNuNfnay6cHeKpEmBEZ5NoTxJofZbKAgA6e2"
    let method: [String: Any] = [
      "id": query.did + "#" + key, "type": "Multikey", "controller": query.did,
      "publicKeyMultibase": key,
    ]
    let valid: [String: Any] = [
      "id": query.did, "status": "Active", "alsoKnownAs": ["did:key:" + key],
      "verificationMethod": [method],
    ]
    let result = try EmailContactLookup.decode(
      JSONSerialization.data(withJSONObject: valid), status: 200, query: query, directory: directory
    )
    guard case .found(let identity) = result else { return XCTFail("Expected account") }
    XCTAssertEqual(identity.email, "alice@example.com")
    XCTAssertEqual(identity.rootDid, "did:key:" + key)
    XCTAssertEqual(identity.directory, directory)
    for (field, value) in [
      ("id", "did:web:other" as Any), ("status", "Registered"), ("deactivated", true),
      ("alsoKnownAs", ["did:key:zOther"]), ("verificationMethod", []),
    ] {
      var invalid = valid
      invalid[field] = value
      XCTAssertThrowsError(
        try EmailContactLookup.decode(
          JSONSerialization.data(withJSONObject: invalid), status: 200, query: query,
          directory: directory), field)
    }
    XCTAssertThrowsError(
      try EmailContactLookup.decode(
        Data("not JSON".utf8), status: 200, query: query, directory: directory))
    for (status, expected) in [
      (202, EmailContactResult.pending), (404, .notFound), (410, .suspended),
    ] {
      XCTAssertEqual(
        try EmailContactLookup.decode(Data(), status: status, query: query, directory: directory),
        expected)
    }
    for status in [204, 301, 401, 429, 500, 502] {
      XCTAssertThrowsError(
        try EmailContactLookup.decode(Data(), status: status, query: query, directory: directory))
    }
  }

  @MainActor
  func testRealHTTPStatusEncodingAndRedirectRefusal() async throws {
    let server = Process()
    server.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
    server.arguments = ["-u", "-c", Self.server]
    let output = Pipe()
    server.standardOutput = output
    server.standardError = FileHandle.nullDevice
    try server.run()
    defer {
      server.terminate()
      server.waitUntilExit()
    }
    let line = String(decoding: output.fileHandleForReading.availableData, as: UTF8.self)
      .trimmingCharacters(in: .whitespacesAndNewlines)
    let port = try XCTUnwrap(Int(line))
    let client = EmailContactLookup(directory: URL(string: "http://127.0.0.1:\(port)")!)
    let result = try await client.lookup(" TAG+Alice@Example.com ")
    guard case .found(let identity) = result else { return XCTFail("Expected account") }
    XCTAssertEqual(identity.email, "tag+alice@example.com")
    for (email, expected) in [
      ("pending", EmailContactResult.pending), ("missing", .notFound), ("suspended", .suspended),
    ] {
      let actual = try await client.lookup(email + "@example.com")
      XCTAssertEqual(actual, expected)
    }
    for email in ["redirect", "limited", "offline", "malformed"] {
      do {
        _ = try await client.lookup(email + "@example.com")
        XCTFail("Expected rejection of \(email)")
      } catch {}
    }
  }

  private static let server = #"""
    import http.server, json
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args): pass
        def do_GET(self):
            assert not self.headers.get('Authorization') and not self.headers.get('Cookie')
            local = self.path.split('/')[3]
            if local == 'redirect':
                self.send_response(302); self.send_header('Location', '/customer/example.com/tag%2Balice/did.json'); self.end_headers(); return
            status = {'pending':202, 'missing':404, 'suspended':410, 'limited':429, 'offline':503}.get(local, 200)
            self.send_response(status); self.send_header('Content-Type', 'application/json'); self.end_headers()
            if status != 200: return
            if local == 'malformed': self.wfile.write(b'{}'); return
            assert local == 'tag%2Balice', self.path
            did = 'did:web:127.0.0.1%3A' + str(self.server.server_port) + ':customer:example.com:' + local
            key = 'z6MkvWgxPPHxXNuNfnay6cHeKpEmBEZ5NoTxJofZbKAgA6e2'
            self.wfile.write(json.dumps({'id':did, 'status':'Active', 'alsoKnownAs':['did:key:' + key], 'verificationMethod':[{'id':did+'#'+key, 'type':'Multikey', 'controller':did, 'publicKeyMultibase':key}]}).encode())
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    print(server.server_port, flush=True)
    server.serve_forever()
    """#
}
