import Foundation
import XCTest

@testable import HarnessCore

final class BrowserCallbackTests: XCTestCase {
  func testHTTPFramingAndFormEscaping() throws {
    let body = "authorize=YWJj%2B%2F%3D&state=test"
    let head =
      "POST / HTTP/1.1\r\nHost: 127.0.0.1:1234\r\nContent-Length: \(body.utf8.count)\r\n\r\n"
    XCTAssertNil(try CallbackRequest.parse(Data((head + String(body.dropLast())).utf8)))
    let request = try XCTUnwrap(CallbackRequest.parse(Data((head + body).utf8)))
    XCTAssertEqual(request.form?["authorize"], "YWJj+/=")
    XCTAssertThrowsError(try CallbackRequest.parse(Data((head + body + "extra").utf8)))
    XCTAssertThrowsError(
      try CallbackRequest.parse(Data("POST / HTTP/1.1\r\nContent-Length: -1\r\n\r\n".utf8)))
    XCTAssertThrowsError(
      try CallbackRequest.parse(
        Data("POST / HTTP/1.1\r\nContent-Length: 0\r\nContent-Length: 1\r\n\r\n".utf8)))
    XCTAssertThrowsError(
      try CallbackRequest.parse(Data("POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n".utf8)))
    XCTAssertThrowsError(try CallbackRequest.parse(Data(repeating: 65, count: 262145)))
  }

  @MainActor
  func testCallbackRejectsForeignOriginAndWrongStateThenDeliversOnce() async throws {
    let callback = BrowserCallback()
    let url = try await callback.start(timeout: .seconds(10))
    defer { callback.cancel() }
    XCTAssertEqual(url.host, "127.0.0.1")
    let (page, _) = try await URLSession.shared.data(from: url)
    let html = try XCTUnwrap(String(data: page, encoding: .utf8))
    let start = try XCTUnwrap(html.range(of: "nonce.value = '"))
    let nonce = String(html[start.upperBound...].prefix(while: { $0 != "'" }))
    let origin = String(url.absoluteString.dropLast())
    for (requestOrigin, state) in [
      ("https://attacker.example", nonce), (origin, "wrong"), ("null", nonce),
    ] {
      var request = URLRequest(url: url)
      request.httpMethod = "POST"
      request.setValue(requestOrigin, forHTTPHeaderField: "Origin")
      request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
      request.httpBody = Data("authorize=aGVsbG8%3D&state=\(state)".utf8)
      let (_, response) = try await URLSession.shared.data(for: request)
      XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 403)
    }
    var request = URLRequest(url: url)
    request.httpMethod = "POST"
    request.setValue("null", forHTTPHeaderField: "Origin")
    request.setValue("same-origin", forHTTPHeaderField: "Sec-Fetch-Site")
    request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
    request.httpBody = Data("authorize=aGVsbG8%3D&state=\(nonce)".utf8)
    let (_, response) = try await URLSession.shared.data(for: request)
    XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200)
    let received = try await callback.receive()
    XCTAssertEqual(received, Data("hello".utf8))
  }

  @MainActor
  func testCancellationAndTimeoutReleaseWaiter() async throws {
    let early = BrowserCallback()
    early.cancel()
    do {
      _ = try await early.start()
      XCTFail("Cancelled attempts cannot open a listener")
    } catch { XCTAssertTrue(error is CancellationError) }
    let cancelled = BrowserCallback()
    _ = try await cancelled.start()
    cancelled.cancel()
    do {
      _ = try await cancelled.receive()
      XCTFail("Cancellation should fail")
    } catch { XCTAssertTrue(error is CancellationError) }
    let expired = BrowserCallback()
    _ = try await expired.start(timeout: .milliseconds(100))
    do {
      _ = try await expired.receive()
      XCTFail("Timeout should fail")
    } catch { XCTAssertTrue(error.localizedDescription.contains("timed out")) }
  }
}
