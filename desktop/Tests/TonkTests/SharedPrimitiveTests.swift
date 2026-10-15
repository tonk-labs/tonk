import HarnessCore
import WebKit
import XCTest

@MainActor
final class SharedPrimitiveTests: XCTestCase {
  func testGeneratedWorkerAdapterRunsInWebKitWithUnchangedResultsAndNoReplay() async throws {
    let webView = WKWebView()
    webView.loadHTMLString("<input id='draft' value='keep'>", baseURL: nil)
    for _ in 0..<100 {
      if !webView.isLoading { break }
      try await Task.sleep(for: .milliseconds(20))
    }
    let script = """
      const draft = window.document.getElementById('draft');
      let requests = [];
      async function fetch(path, options) {
        requests.push({path, body: options.body});
        if (lost) throw new Error('private transport detail');
        return new Response(JSON.stringify({revision_before:null, revision_after:null,
          commits:{claims:0}, matches_after:[{results:[{this:'id:test'}]}]}),
          {headers:{'content-type':'application/json'}});
      }
      async function execute() { \(TonkAgentContext.workerScript) }
      const result = await execute();
      return {requests, result, sameNode: draft === window.document.getElementById('draft'), value: draft.value};
      """
    for tool in TonkToolCatalog.primitiveNames {
      for lost in [false, true] {
        let result =
          try await webView.callAsyncJavaScript(
            script,
            arguments: [
              "subject": "did:key:z123", "tool": tool, "lost": lost,
              "argumentsJSON": "{\"document\":\"thing!:\\n  this: id:test\"}",
            ], in: nil, contentWorld: .page) as! [String: Any]
        let requests = result["requests"] as! [[String: Any]]
        XCTAssertEqual(requests.count, 1)
        XCTAssertEqual(
          requests[0]["path"] as? String,
          "/api/repository/did%3Akey%3Az123/branch/main/evaluate?transact=\(tool == "tonk_evaluate")"
        )
        XCTAssertEqual(requests[0]["body"] as? String, "thing!:\n  this: id:test")
        XCTAssertEqual(result["sameNode"] as? Bool, true)
        XCTAssertEqual(result["value"] as? String, "keep")
        let outcome = result["result"] as! [String: Any]
        if lost {
          let error = try XCTUnwrap(outcome["error"] as? String)
          XCTAssertFalse(error.contains("private"))
          if tool == "tonk_evaluate" { XCTAssertTrue(error.contains("Do not repeat")) }
        } else {
          let text = try XCTUnwrap(outcome["response"] as? String)
          let value = try JSONDecoder().decode(JSONValue.self, from: Data(text.utf8))
          XCTAssertEqual(
            value["matches_after"].array.first?["results"].array.first?["this"], .string("id:test"))
          XCTAssertEqual(value["commits"]["claims"], .number(0))
          XCTAssertEqual(value["matches"], .null)
        }
      }
    }
    for input in ["{\"document\":\"thing:\",\"space\":\"other\"}", "{\"document\":\" \"}"] {
      let result =
        try await webView.callAsyncJavaScript(
          script,
          arguments: [
            "subject": "did:key:z123", "tool": "tonk_evaluate", "lost": false,
            "argumentsJSON": input,
          ], in: nil, contentWorld: .page) as! [String: Any]
      XCTAssertEqual((result["requests"] as! [Any]).count, 0)
      XCTAssertNotNil((result["result"] as? [String: Any])?["error"])
    }
  }
}
