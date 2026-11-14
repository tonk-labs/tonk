import WebKit
import XCTest

@testable import Tonk

@MainActor
final class RuntimeCatalogObservationTests: XCTestCase {
  func testStreamInvalidatesAfterDelayedAndSplitEventsWithoutDuplicatingSubscription() async throws {
    let webView = WKWebView()
    webView.loadHTMLString("<p>Catalog fixture</p>", baseURL: nil)
    for _ in 0..<100 {
      if !webView.isLoading { break }
      try await Task.sleep(for: .milliseconds(20))
    }
    let result = try await webView.callAsyncJavaScript(
      """
      const notifications = [];
      const requests = [];
      const webkit = {messageHandlers: {catalogChanged: {postMessage: value => notifications.push(value)}}};
      const catalogGeneration = 'fixture';
      const path = '/api/profile/branch/account%2Ftest/query';
      const profilePrefix = '/api/repository/profile:tonk/branch/';
      const query = {predicate: {with: {}}};
      let stream;
      const fetch = async (path, options) => {
        requests.push({path, accept: options.headers.Accept});
        return {ok: true, headers: new Headers({'content-type': 'text/event-stream'}),
          body: new ReadableStream({start(controller) { stream = controller; }})};
      };
      const install = () => { \(RuntimeCatalogObservation.script) };
      install();
      await new Promise(resolve => setTimeout(resolve, 0));
      const send = value => stream.enqueue(new TextEncoder().encode(value));
      send('data: []\\n\\n');
      await new Promise(resolve => setTimeout(resolve, 0));
      install();
      send('data: [{"fields":');
      await new Promise(resolve => setTimeout(resolve, 0));
      const beforeComplete = notifications.length;
      send('{}}]\\n\\n');
      await new Promise(resolve => setTimeout(resolve, 0));
      stream.close();
      await new Promise(resolve => setTimeout(resolve, 0));
      return {requests, notifications, beforeComplete, cleared: !window.__tonkCatalogStream};
      """, arguments: [:], in: nil, contentWorld: .page) as! [String: Any]
    let requests = result["requests"] as! [[String: String]]
    XCTAssertEqual(requests, [["path": "/api/repository/profile:tonk/branch/account%2Ftest/query",
      "accept": "text/event-stream"]])
    XCTAssertEqual(result["beforeComplete"] as? Int, 1)
    XCTAssertEqual(result["notifications"] as? [[String: String]], [
      ["generation": "fixture", "failed": "false"],
      ["generation": "fixture", "failed": "false"],
      ["generation": "fixture", "failed": "true"],
    ])
    XCTAssertEqual(result["cleared"] as? Bool, true)
  }
}
