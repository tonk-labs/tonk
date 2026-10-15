import HarnessCore
import WebKit
import XCTest

@testable import Tonk

@MainActor
final class RuntimeInspectionTests: XCTestCase {
  func testNativeFabFlagIsAvailableBeforePageScriptsOnEveryLoad() async throws {
    let runtime = RuntimeModel()
    let webView = runtime.webView
    webView.navigationDelegate = nil
    for _ in 0..<2 {
      webView.loadHTMLString(
        "<script>window.flagAtBoot = window.__tonkHideFab === true;</script>", baseURL: nil)
      for _ in 0..<100 {
        if !webView.isLoading { break }
        try await Task.sleep(for: .milliseconds(20))
      }
      let flag = try await webView.callAsyncJavaScript(
        "return window.flagAtBoot === true;", arguments: [:], in: nil, contentWorld: .page)
      XCTAssertEqual(flag as? Bool, true)
    }
  }

  func testInspectionRequiresCatalogMembershipAndMatchingOpenPreview() async throws {
    let space = try XCTUnwrap(
      TonkSpace.decodeCatalog(
        Data("[{\"subject\":\"did:key:z123\",\"name\":\"Fixture\"}]".utf8)
      ).first)
    let runtime = RuntimeModel()
    runtime.loading = false
    runtime.catalogLoaded = true
    runtime.catalogBranch = "main"
    for stage in 0..<3 {
      if stage == 1 { runtime.spaces = [space] }
      if stage == 2 { runtime.selectedSpace = space }
      do {
        _ = try await runtime.performBuildTool(
          space, tool: "tonk_inspect_view", arguments: .object([:]))
        XCTFail("Must reject missing membership, selection, or matching loaded URL")
      } catch {
        XCTAssertTrue(error is HarnessError)
      }
    }
  }

  func testSandboxedFramesAndControlState() async throws {
    let inspector = RuntimeInspection()
    let configuration = WKWebViewConfiguration()
    inspector.install(on: configuration.userContentController)
    let webView = WKWebView(
      frame: CGRect(x: 0, y: 0, width: 800, height: 600), configuration: configuration)
    webView.loadHTMLString(
      """
      <html><body><h1>Outer fixture</h1>
      <script>setTimeout(() => { throw new Error("outer failure"); }, 100);</script>
      <tonk-display id="tracked">Before observation</tonk-display>
      <script>
        document.getElementById('tracked').inspectCompletion = function() {
          this.textContent = 'Same-task observation';
          return {status: 'complete', textContent: this.textContent, displays: []};
        };
      </script>
      <div hidden>HIDDEN_SENTINEL</div>
      <input type="password" value="PASSWORD_SENTINEL">
      <iframe sandbox="allow-scripts" srcdoc="<html><body><label><input type='checkbox' checked>Pack food</label><button>Save</button><script>setTimeout(() => { throw new Error('fixture failure'); }, 100);</script></body></html>"></iframe>
      </body></html>
      """, baseURL: URL(string: "https://staging.tonk.xyz"))
    var result: [String: Any] = [:]
    for _ in 0..<50 {
      result = try await inspector.snapshot(in: webView)
      let frames = result["frames"] as? [[String: Any]] ?? []
      if frames.count >= 2, String(describing: frames).contains("outer failure"),
        String(describing: frames).contains("Script error.")
      {
        break
      }
      try await Task.sleep(for: .milliseconds(100))
    }
    let frames = try XCTUnwrap(result["frames"] as? [[String: Any]])
    XCTAssertEqual(frames.count, 2)
    XCTAssertTrue(result["renderedRevision"] is NSNull)
    XCTAssertEqual(result["revisionTracking"] as? String, "unavailable")
    let encoded = String(
      decoding: try JSONSerialization.data(withJSONObject: result), as: UTF8.self)
    let main = try XCTUnwrap(frames.first { $0["mainFrame"] as? Bool == true })
    let completion = try XCTUnwrap(main["renderCompletion"] as? [String: Any])
    let observations = try XCTUnwrap(completion["observations"] as? [[String: Any]])
    XCTAssertEqual(observations.first?["textContent"] as? String, "Same-task observation")
    XCTAssertTrue(encoded.contains("Outer fixture"))
    XCTAssertTrue(encoded.contains("Pack food"))
    XCTAssertTrue(encoded.contains("outer failure"), encoded)
    XCTAssertTrue(encoded.contains("Script error."), encoded)
    XCTAssertFalse(encoded.contains("HIDDEN_SENTINEL"))
    XCTAssertFalse(encoded.contains("PASSWORD_SENTINEL"))
    let controls = frames.flatMap { $0["controls"] as? [[String: Any]] ?? [] }
    let checkbox = try XCTUnwrap(controls.first { $0["type"] as? String == "checkbox" })
    XCTAssertEqual(checkbox["checked"] as? Bool, true)
    inspector.reset()
    let cleared = try await inspector.snapshot(in: webView)
    XCTAssertEqual((cleared["frames"] as? [[String: Any]])?.count, 0)
  }
}
