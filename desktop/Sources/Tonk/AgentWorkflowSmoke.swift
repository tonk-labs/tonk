import AppKit
import HarnessCore

/// Opt-in driver using the real native MCP, worker and renderer in a fresh local profile.
@MainActor
enum AgentWorkflowSmoke {
  static func run(model: HarnessModel, runtime: RuntimeModel) async {
    let args = ProcessInfo.processInfo.arguments
    func option(_ key: String) -> String? {
      guard let i = args.firstIndex(of: key), args.indices.contains(i + 1) else { return nil }
      return args[i + 1]
    }
    guard RuntimeLocation.deployment == .local, option("--web-data-id") != nil,
      let reportPath = option("--report"), let fixturePath = option("--workflow-fixture")
    else {
      NSApp.terminate(nil)
      return
    }
    let reportURL = URL(fileURLWithPath: reportPath)
    var report: [String: Any] = [
      "passed": false, "scope": "Local native MCP, WebKit worker and renderer",
    ]
    var checks: [String] = []
    func save() {
      report["checks"] = checks
      if let data = try? JSONSerialization.data(
        withJSONObject: report, options: [.prettyPrinted, .sortedKeys])
      {
        try? data.write(to: reportURL, options: .atomic)
      }
    }
    func check(_ condition: Bool, _ name: String) throws {
      guard condition else { throw HarnessError.message(name) }
      checks.append(name)
      save()
    }
    func wait(_ name: String, _ body: () async throws -> Bool) async throws {
      var detail = ""
      for _ in 0..<150 {
        do { if try await body() { return } } catch { detail = error.localizedDescription }
        try await Task.sleep(for: .milliseconds(200))
      }
      throw HarnessError.message("Timed out: " + name + ". " + detail)
    }
    func beginTurn() {
      model.saved.conversation.threadID = "workflow-local"
      model.turnID = "workflow-turn"
      model.busy = true
    }
    let tools = SpaceTools.agentDefinitions(includeCLI: false).map { tool in
      JSONValue.object([
        "name": tool["name"], "description": tool["description"],
        "inputSchema": tool["inputSchema"],
      ])
    }
    let bridge = LocalRuntimeBridge(allowsWrites: true, agentTools: tools) { name, arguments in
      report["toolReadiness"] = [
        "tool": name, "loading": runtime.loading, "catalogLoading": runtime.catalogLoading,
        "catalogLoaded": runtime.catalogLoaded, "cached": runtime.showingCachedCatalog,
        "catalogError": runtime.catalogError ?? "", "signInPending": runtime.signInPending,
        "branch": runtime.catalogBranch ?? "", "url": runtime.spaceWebView.url?.absoluteString ?? "",
        "member": runtime.spaces.contains { $0.subject == model.saved.conversation.space?.subject },
      ]
      return await model.callSpaceTool(
        .object([
          "threadId": .string("workflow-local"), "turnId": .string("workflow-turn"),
          "namespace": .null, "tool": .string(name), "arguments": arguments,
        ]))
    }
    defer {
      bridge.stop()
      model.busy = false
      model.persist()
      model.shutdown()
      NSApp.terminate(nil)
    }
    let watchdog = Task {
      try? await Task.sleep(for: .seconds(150))
      guard !Task.isCancelled else { return }
      report["error"] = "Workflow deadline exceeded"
      save()
      NSApp.terminate(nil)
    }
    defer { watchdog.cancel() }
    do {
      save()
      try await wait("worker and catalog ready") {
        let probe = try await runtime.probe()
        report["startupProbe"] = probe
        report["catalogError"] = runtime.catalogError ?? ""
        save()
        guard probe["health"] as? Bool == true else { return false }
        await runtime.refreshSpaces()
        return !runtime.loading && runtime.catalogLoaded && runtime.catalogError == nil
          && probe["health"] as? Bool == true
      }
      report["worker"] = try await runtime.accountScript("return await api('/api/health');")
      let connection = try await bridge.start()
      var sequence = 0
      func rpc(_ method: String, _ params: JSONValue = .object([:])) async throws -> JSONValue {
        sequence += 1
        var request = URLRequest(url: URL(string: connection["url"].string! + "/mcp")!)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue(
          "Bearer " + connection["token"].string!, forHTTPHeaderField: "Authorization")
        request.httpBody = try JSONEncoder().encode(
          JSONValue.object([
            "jsonrpc": .string("2.0"), "id": .number(Double(sequence)),
            "method": .string(method), "params": params,
          ]))
        let (data, response) = try await URLSession.shared.data(for: request)
        guard (response as? HTTPURLResponse)?.statusCode == 200 else {
          throw HarnessError.message("MCP transport failed")
        }
        let envelope = try JSONDecoder().decode(JSONValue.self, from: data)
        guard envelope["error"] == .null else {
          throw HarnessError.message("MCP protocol rejected " + method)
        }
        return envelope["result"]
      }
      func call(_ name: String, _ arguments: JSONValue = .object([:])) async throws -> JSONValue {
        if name != "tonk_guide", let space = model.saved.conversation.space {
          try await wait("attached space ready before " + name) {
            try runtime.requireSpaceReady(space)
            return true
          }
        }
        let result = try await rpc(
          "tools/call", .object(["name": .string(name), "arguments": arguments]))
        let text = result["content"].array.first?["text"].string ?? ""
        guard result["isError"].bool != true else { throw HarnessError.message(name + ": " + text) }
        return (try? JSONDecoder().decode(JSONValue.self, from: Data(text.utf8))) ?? .string(text)
      }
      func query(_ document: String) async throws -> JSONValue {
        try await call("tonk_query", .object(["document": .string(document)]))
      }
      func evaluate(_ document: String) async throws {
        _ = try await call("tonk_evaluate", .object(["document": .string(document)]))
      }
      func rendered(_ text: String) async throws -> JSONValue {
        var found: JSONValue = .null
        try await wait("rendered " + text) {
          found = try await call("tonk_inspect_view")
          return found["frames"].array.contains { $0["text"].string?.contains(text) == true }
        }
        return found
      }
      let initialized = try await rpc("initialize")
      try check(
        initialized["instructions"].string == TonkAgentContext.instructions,
        "MCP exposes canonical context")
      let advertised = try await rpc("tools/list")
      try check(
        advertised["tools"].array.map { $0["name"] } == tools.map { $0["name"] },
        "MCP exposes native shared tool list")
      beginTurn()
      for topic in ["agent", "notation", "views", "events"] {
        let guide = try await call("tonk_guide", .object(["topic": .string(topic)]))
        let resource = try await rpc(
          "resources/read", .object(["uri": .string("tonk://guide/" + topic)]))
        try check(
          guide["text"] == resource["contents"].array.first?["text"],
          "Guide/resource parity: " + topic)
      }
      if args.contains("--workflow-reopen") {
        guard let space = model.saved.conversation.space else {
          throw HarnessError.message("Saved space missing")
        }
        model.openedSpace = space
        runtime.openSpace(space)
        try await wait("reopened space ready") {
          !runtime.loading && !runtime.spaceLoading && runtime.catalogLoaded && runtime.spaceWebView.url == space.url
        }
        let rows = try await query("integration-task:\n  this: urn:test:task:one\n")
        try check(
          rows["matches_after"].array.first?["results"].array.first?["fields"]["done"]
            == .bool(true),
          "Checkbox write survives process restart")
        _ = try await rendered("Updated first task")
        let name = try await query("tonk/repository:\n  this: \(space.subject)\n")
        try check(
          name["matches_after"].array.first?["results"].array.first?["fields"]["name"]
            == .string("Workflow renamed"),
          "Space rename survives process restart")
        report["space"] = space.subject
      } else {
        let before = runtime.spaces.count
        _ = try await call(
          "tonk_propose_space",
          .object(["name": .string("Workflow test"), "reason": .string("Test shared authoring")]))
        try check(
          model.saved.conversation.spaceProposal != nil && runtime.spaces.count == before,
          "Native creation proposal persists without creating a space")
        model.busy = false
        model.dismissSpaceProposal()
        // Local bootstrap is explicit. Account-backed approval/sync is a separate live check.
        let created = try await runtime.accountScript(
          """
          const response = await fetch('/api/repository/' + encodeURIComponent(name), {
            method:'PUT', headers:{'Content-Type':'application/json'}, body:JSON.stringify({branch:{main:{}}})
          });
          if (!response.ok) throw new Error('Local space creation failed');
          return await response.json();
          """, arguments: ["name": "Workflow " + UUID().uuidString])
        guard let subject = created["subject"] as? String else {
          throw HarnessError.message("Created space has no subject")
        }
        try await wait("created space in native catalog") {
          await runtime.refreshSpaces()
          return runtime.spaces.contains { $0.subject == subject }
        }
        guard let space = runtime.spaces.first(where: { $0.subject == subject }) else {
          throw HarnessError.message("Created space missing from native catalog")
        }
        report["space"] = subject
        model.attachSpace(space)
        model.openedSpace = space
        runtime.openSpace(space)
        try await wait("selected space ready") {
          !runtime.loading && !runtime.spaceLoading && runtime.catalogLoaded && runtime.spaceWebView.url == space.url
        }
        beginTurn()
        try check(
          model.saved.conversation.space?.subject == subject,
          "Space created, catalogued, selected and attached")
        let schemas = try await query("concept:\n")
        try check(!schemas["matches_after"].array.isEmpty, "Schema discovery through query")
        try await evaluate(try String(contentsOfFile: fixturePath, encoding: .utf8))
        let rows = try await query("integration-task:\n")
        try check(
          rows["matches_after"].array.first?["results"].array.count == 3,
          "Schema and three records built through evaluate")
        let views = try await call("tonk_guide", .object(["topic": .string("views")]))
        guard let manual = views["text"].string else {
          throw HarnessError.message("Views guide missing")
        }
        func example(_ name: String, in text: String? = nil) throws -> String {
          let fence = String(repeating: String(UnicodeScalar(96)), count: 3)
          guard
            let section = (text ?? manual).components(
              separatedBy: "<!-- tested-example: " + name + " -->"
            )
            .dropFirst().first,
            let code = section.components(separatedBy: fence + "yaml tonk=parse\n").dropFirst()
              .first?.components(separatedBy: fence).first
          else { throw HarnessError.message("Guide example missing: " + name) }
          return code
        }
        let view = try example("task-views").replacingOccurrences(
          of: "checked={done}",
          with: "checked={done} onchange=workflow-toggle data-task={this}")
        try await evaluate(
          """
          command!: &workflow-toggle
            description: Change task completion from its checkbox.
            with:
              task:
                description: The task to update.
                the: dom.event.current-target.dataset/task
                as: entity
              done:
                description: Whether the task is complete.
                the: dom.event.current-target/checked
                as: boolean
          rule!:
            assert!: integration-task
            when:
              - assert: workflow-toggle
                where: {task: ?this, done: ?done}
              - assert: integration-task
                where: {this: ?this, title: ?title}
          """ + "\n" + view + "\n" + (try example("space-home")))
        model.openedSpace = space
        runtime.openSpace(space)
        try await wait("home ready") { !runtime.spaceLoading && runtime.spaceWebView.url == space.url }
        let inspection = try await rendered("First task")
        report["inspection"] = try JSONSerialization.jsonObject(
          with: JSONEncoder().encode(inspection))
        let controls = inspection["frames"].array.filter { $0["mainFrame"] == .bool(false) }
          .flatMap { $0["controls"].array }
        try check(
          controls.filter { $0["type"] == .string("checkbox") }.count == 3,
          "Guide-built view renders three real checkboxes")
        _ = try await query(
          "integration-task!:\n  this: urn:test:task:one\n  title: \"Must not persist\"\n")
        let dryRun = try await query("integration-task:\n  this: urn:test:task:one\n")
        try check(
          dryRun["matches_after"].array.first?["results"].array.first?["fields"]["title"]
            == .string("First task"),
          "Query dry-run does not commit assertions")
        try await evaluate(
          "integration-task!:\n  this: urn:test:task:one\n  title: \"Updated first task\"\n")
        _ = try await rendered("Updated first task")
        try check(true, "Record update renders without navigation")
        try await runtime.inspection.clickWorkflowCheckbox(
          in: runtime.spaceWebView, title: "Updated first task")
        try await wait("checkbox event persisted") {
          let result = try await query("integration-task:\n  this: urn:test:task:one\n")
          return result["matches_after"].array.first?["results"].array.first?["fields"]["done"]
            == .bool(true)
        }
        try check(true, "Rendered checkbox event persists through its command and rule")
        let spacesGuide = try await call("tonk_guide", .object(["topic": .string("spaces")]))
        let rename = try example("rename-space", in: spacesGuide["text"].string)
          .replacingOccurrences(of: "urn:example:authorized-space", with: subject)
          .replacingOccurrences(of: "New space name", with: "Workflow renamed")
        try await evaluate(rename)
        try await wait("rename visible in native catalog") {
          await runtime.refreshSpaces()
          return runtime.spaces.first(where: { $0.subject == subject })?.title == "Workflow renamed"
        }
        try check(true, "Evaluate renames the space and updates the native picker")
        let invalid = try await rpc(
          "tools/call",
          .object([
            "name": .string("tonk_query"),
            "arguments": .object(["document": .string("not-valid: [")]),
          ]))
        try check(invalid["isError"] == .bool(true), "Invalid notation returns a tool error")
        let override = try await rpc(
          "tools/call",
          .object([
            "name": .string("tonk_evaluate"),
            "arguments": .object(["document": .string("concept:"), "space": .string("other")]),
          ]))
        try check(override["isError"] == .bool(true), "Tool cannot override the attached space")
        try await evaluate("library/install!:\n  component: tonk:library/notebook\n  time: 1.0\n")
        try await wait("library installed") {
          let notebook = try await query("concept:\n  name: \"notebook/named\"\n")
          return !(notebook["matches_after"].array.first?["results"].array.isEmpty ?? true)
        }
        try check(true, "Library installation remains available through evaluate")
        let otherCreated = try await runtime.accountScript(
          """
          const response = await fetch('/api/repository/' + encodeURIComponent(name), {
            method:'PUT', headers:{'Content-Type':'application/json'}, body:JSON.stringify({branch:{main:{}}})
          });
          if (!response.ok) throw new Error('Second local space creation failed');
          return await response.json();
          """, arguments: ["name": "Other workflow " + UUID().uuidString])
        guard let otherSubject = otherCreated["subject"] as? String else {
          throw HarnessError.message("Second space has no subject")
        }
        try await wait("second space catalogued") {
          await runtime.refreshSpaces()
          return runtime.spaces.contains { $0.subject == otherSubject }
        }
        guard let other = runtime.spaces.first(where: { $0.subject == otherSubject }) else {
          throw HarnessError.message("Second space absent")
        }
        model.busy = false
        model.attachSpace(other)
        model.openedSpace = other
        runtime.openSpace(other)
        beginTurn()
        let isolated = try await query("concept:\n  name: \"integration-task\"\n")
        try check(
          isolated["matches_after"].array.first?["results"].array.isEmpty == true,
          "Switching spaces changes the tool target without leaking the first schema")
        model.busy = false
        model.attachSpace(space)
        model.openedSpace = space
        runtime.openSpace(space)
        beginTurn()
        _ = try await rendered("Updated first task")
        try check(true, "Switching back restores the authored view")
        model.busy = false
        if let renamed = runtime.spaces.first(where: { $0.subject == subject }) {
          model.saved.conversation.space = renamed
        }
        model.persist()
      }
      report["passed"] = true
    } catch {
      report["error"] =
        (error as NSError).userInfo["WKJavaScriptExceptionMessage"] as? String
        ?? error.localizedDescription
    }
    save()
  }
}
