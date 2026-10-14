import AppKit
import Foundation

@MainActor
enum SmokeTest {
  /// Read-only sync investigation, independent of model login and space mounting.
  static func diagnoseSync(runtime: RuntimeModel) async {
    let arguments = ProcessInfo.processInfo.arguments
    guard let index = arguments.firstIndex(of: "--diagnose-sync"),
      arguments.indices.contains(index + 1),
      let reportIndex = arguments.firstIndex(of: "--report"),
      arguments.indices.contains(reportIndex + 1)
    else {
      NSApp.terminate(nil)
      return
    }
    let target = URL(fileURLWithPath: arguments[reportIndex + 1])
    var report: [String: Any] = [:]
    func save() {
      if let data = try? JSONSerialization.data(
        withJSONObject: report, options: [.prettyPrinted, .sortedKeys])
      {
        try? data.write(to: target, options: .atomic)
      }
    }
    save()
    for _ in 0..<20 {
      if (try? await runtime.accountScript("return {ready: true};")) != nil { break }
      try? await Task.sleep(for: .seconds(1))
    }
    let steps = [
      (
        "identity",
        """
        const root = await api('/api/identity/root');
        return {status: root.status, rootDid: root.rootDid, deviceDid: root.deviceDid,
          delegationCid: root.delegationCid};
        """
      ),
      (
        "deviceGrants",
        """
        const root = await api('/api/identity/root');
        const variable = name => ({'?': {name}});
        const field = (the, as = 'Text') => ({the, as, cardinality: 'one'});
        const info = await api('/api/profile/repository');
        const roster = await api('/api/profiles');
        const branches = [...new Set([...Object.keys(info.branch || {}), roster.active])].filter(Boolean);
        function base32(bytes) {
          const alphabet = 'abcdefghijklmnopqrstuvwxyz234567';
          let bits = 0, value = 0, out = 'b';
          for (const byte of bytes) {
            value = (value << 8) | byte; bits += 8;
            while (bits >= 5) { bits -= 5; out += alphabet[(value >>> bits) & 31]; }
          }
          if (bits) out += alphabet[(value << (5 - bits)) & 31];
          return out;
        }
        let blobs;
        try {
          let directory = await navigator.storage.getDirectory();
          for (const part of ['profile', info.name, 'blob']) directory = await directory.getDirectoryHandle(part);
          blobs = directory;
        } catch {}
        const results = [];
        for (const branch of branches) {
          const rows = await api('/api/profile/branch/' + encodeURIComponent(branch) + '/query', {
            predicate: {with: {audience: field('dialog.ucan/audience'), issuer: field('dialog.ucan/issuer'), subject: field('dialog.ucan/subject')}},
            terms: {this: variable('this'), audience: variable('audience'), issuer: variable('issuer'), subject: variable('subject')}
          });
          const grants = [];
          for (const row of rows.filter(row => row.fields?.audience === root.deviceDid)) {
            const metadata = {...row.fields};
            if (blobs) {
              try {
                const file = await (await blobs.getFileHandle(row.this.replace(/^blob:/, ''))).getFile();
                const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', await file.arrayBuffer()));
                metadata.cid = base32(new Uint8Array([1, 113, 18, 32, ...digest]));
                metadata.current = metadata.cid === root.delegationCid;
              } catch (error) { metadata.fingerprintError = error.name; }
            }
            grants.push(metadata);
          }
          results.push({branch, total: rows.length, grants});
        }
        return {branches, active: roster.active, blobStoreFound: !!blobs, results};
        """
      ),
      (
        "spaceSync",
        """
        const response = await fetch('/api/repository/' + encodeURIComponent(subject) + '/branch/main/sync/status', {
          signal: AbortSignal.timeout(15000)
        });
        const body = await response.json();
        return {httpStatus: response.status, state: body.state, error: body.error};
        """
      ),
      (
        "health",
        """
        const health = await api('/api/health');
        return {build: health.build, worker: health.worker, workerWasm: health.workerWasm,
          log: (health.log || []).filter(entry => /revoked|revocation|sign-out|failed.*pull|pull.*failed/i.test(JSON.stringify(entry))).slice(-20)};
        """
      ),
    ]
    for (name, script) in steps {
      report["pending"] = name
      save()
      do {
        report[name] = try await runtime.accountScript(
          script, arguments: ["subject": arguments[index + 1]])
      } catch {
        report[name] = [
          "error": (error as NSError).userInfo["WKJavaScriptExceptionMessage"] as? String
            ?? error.localizedDescription
        ]
      }
      report.removeValue(forKey: "pending")
      save()
    }
    NSApp.terminate(nil)
  }

  static func run(model: HarnessModel, runtime: RuntimeModel) async {
    var report: [String: Any] = ["appServer": model.connected, "signedIn": model.signedIn]
    var lastProbe: [String: Any] = [:]
    for _ in 0..<40 {
      if let result = try? await runtime.probe() {
        lastProbe = result
        if result["health"] as? Bool == true && result["mounted"] as? Bool == true
          && runtime.catalogLoaded
        {
          break
        }
      }
      try? await Task.sleep(for: .seconds(1))
    }
    report["catalogLoaded"] = runtime.catalogLoaded
    report["spaceCount"] = runtime.spaces.count
    if let error = runtime.catalogError { report["catalogError"] = error }
    report["runtime"] = lastProbe
    report["passed"] =
      model.connected && lastProbe["health"] as? Bool == true
      && lastProbe["mounted"] as? Bool == true && runtime.catalogLoaded
    if let error = runtime.error { report["runtimeError"] = error }
    if let error = model.error { report["agentError"] = error }
    let arguments = ProcessInfo.processInfo.arguments
    if arguments.contains("--inspect-worker") {
      report["worker"] = try? await runtime.accountScript(
        """
        const registration = await navigator.serviceWorker.getRegistration();
        const health = await api('/api/health');
        const deployed = await api('/version.json');
        return {build: health.build, worker: health.worker, workerWasm: health.workerWasm,
          deployed, controller: navigator.serviceWorker.controller?.scriptURL || '',
          active: registration?.active?.scriptURL || '',
          waiting: registration?.waiting?.scriptURL || ''};
        """)
    }
    if let index = arguments.firstIndex(of: "--inspect-space"),
      arguments.indices.contains(index + 1)
    {
      do {
        guard let space = runtime.spaces.first(where: { $0.id == arguments[index + 1] }) else {
          throw NSError(
            domain: "SmokeTest", code: 1,
            userInfo: [NSLocalizedDescriptionKey: "Requested space is not in the catalog"])
        }
        report["spaceInspection"] = try await runtime.performSpaceTool(space: space, name: nil)
        if arguments.contains("--inspect-sync") {
          report["syncDiagnostic"] = try await runtime.accountScript(
            """
            const root = await api('/api/identity/root');
            const health = await api('/api/health');
            async function status(subject, branch = 'main') {
              const response = await fetch('/api/repository/' + encodeURIComponent(subject) + '/branch/' + encodeURIComponent(branch) + '/sync/status', {
                signal: AbortSignal.timeout(60000)
              });
              const result = await response.json();
              return {httpStatus: response.status, state: result.state, code: result.error?.code,
                message: result.error?.message};
            }
            const sync = await status(subject);
            return {
              identity: {status: root.status, rootDid: root.rootDid, deviceDid: root.deviceDid,
                delegationCid: root.delegationCid},
              build: health.build, worker: health.worker, workerWasm: health.workerWasm,
              sync
            };
            """,
            arguments: ["subject": space.subject])
        }
        if arguments.contains("--inspect-worker") {
          report["handoff"] = try await runtime.accountScript(
            """
            const field = (the) => ({the: 'xyz.tonk.agent-handoff/' + the, as: 'Text', cardinality: 'one'});
            const data = await api('/api/repository/' + encodeURIComponent(subject) + '/branch/main/query', {
              predicate: {with: {status: field('status'), link: field('link')}},
              terms: {this: subject, status: {'?': {name: 'status'}}, link: {'?': {name: 'link'}}}
            });
            const rows = Array.isArray(data) ? data : data.conclusions || [];
            return {rows: rows.map(row => {
              const fields = row.fields || {};
              let kind = 'empty';
              try {
                const url = new URL(fields.link);
                kind = url.hash.startsWith('#tonk-agent-') ? 'agent' : url.searchParams.has('access') ? 'person' : url.pathname.startsWith('/@/') ? 'shortcut' : 'unknown';
              } catch {}
              return {status: fields.status, linkKind: kind};
            })};
            """, arguments: ["subject": space.subject])
        }
        if arguments.contains("--inspect-schema") {
          report["spaceSchema"] = try await runtime.readSpaceSchema(space)
        }
      } catch {
        report["passed"] = false
        report["inspectionError"] =
          (error as NSError).userInfo["WKJavaScriptExceptionMessage"] as? String
          ?? error.localizedDescription
      }
    }
    if let index = arguments.firstIndex(of: "--report"), arguments.indices.contains(index + 1) {
      let target = URL(fileURLWithPath: arguments[index + 1])
      if let data = try? JSONSerialization.data(
        withJSONObject: report, options: [.prettyPrinted, .sortedKeys])
      {
        try? data.write(to: target, options: .atomic)
      }
    }
    model.shutdown()
    NSApp.terminate(nil)
  }
}
