import Foundation
import HarnessCore

@MainActor
extension RuntimeModel {
  /// Explicit local test bootstrap. Never imports a cloud account or retries a write.
  func prepareLocalFixtureIfRequested() async {
    guard RuntimeLocation.deployment == .local,
      ProcessInfo.processInfo.arguments.contains("--seed-local-fixture"), !localFixtureStarted
    else { return }
    localFixtureStarted = true
    do {
      _ = try await accountScript(
        """
        const fixture = await fetch('/__test/direct-build.notation');
        if (!fixture.ok) throw new Error('The local fixture server is unavailable.');
        const document = await fixture.text();
        const created = await fetch('/api/repository/Robin%20Local%20Build%20Test', {
          method: 'PUT', headers: {'Content-Type': 'application/json'},
          body: JSON.stringify({branch: {main: {}}})
        });
        if (!created.ok) throw new Error('Local test space creation failed.');
        const info = await created.json();
        const seeded = await fetch('/api/repository/' + encodeURIComponent(info.subject) + '/branch/main/evaluate', {
          method: 'POST', headers: {'Content-Type': 'text/plain'}, body: document
        });
        if (!seeded.ok) throw new Error('Local fixture initialization failed; do not repeat automatically.');
        return {created: true};
        """)
    } catch {
      self.error = "Local fixture setup failed: " + error.localizedDescription
    }
  }
}
