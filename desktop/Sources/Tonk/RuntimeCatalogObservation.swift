import Foundation
import HarnessCore
import WebKit

/// Read-only invalidation bridge. Catalog data still goes through the normal query.
@MainActor
final class RuntimeCatalogObservation: NSObject, WKScriptMessageHandler {
  weak var model: RuntimeModel?
  var generation = UUID().uuidString

  func userContentController(
    _ userContentController: WKUserContentController, didReceive message: WKScriptMessage
  ) {
    guard message.frameInfo.isMainFrame,
      let url = message.frameInfo.request.url, RuntimeLocation.isEmbedded(url),
      let value = message.body as? [String: String], value["generation"] == generation,
      let model
    else { return }
    if value["failed"] == "true" {
      model.catalogError = "Live space updates stopped. Refresh to reconnect."
    } else {
      Task { await model.refreshSpaces() }
    }
  }

  // api() has already detected the current worker's profile route prefix.
  // Reuse one stream per branch/document; refresh restarts a failed stream.
  static let script = """
    const streamPath = path.replace('/api/profile/branch/', profilePrefix);
    const key = catalogGeneration + ':' + streamPath;
    if (window.__tonkCatalogStream?.key !== key) {
      window.__tonkCatalogStream?.controller.abort();
      const controller = new AbortController();
      const state = {key, controller};
      window.__tonkCatalogStream = state;
      const notify = failed => webkit.messageHandlers.catalogChanged.postMessage({
        generation: catalogGeneration, failed: String(failed)
      });
      void (async () => {
        try {
          const response = await fetch(streamPath, {
            method: 'POST', headers: {'Content-Type': 'application/json', 'Accept': 'text/event-stream'},
            body: JSON.stringify(query), signal: controller.signal
          });
          if (!response.ok || !response.headers.get('content-type')?.includes('text/event-stream'))
            throw new Error('Catalog subscription unavailable');
          const reader = response.body.getReader();
          const decoder = new TextDecoder();
          let buffer = '';
          try {
            while (true) {
              const {value, done} = await reader.read();
              if (done) throw new Error('Catalog subscription closed');
              buffer += decoder.decode(value, {stream: true});
              let boundary;
              while ((boundary = buffer.indexOf('\\n\\n')) !== -1) {
                const event = buffer.slice(0, boundary);
                buffer = buffer.slice(boundary + 2);
                if (event.split('\\n').some(line => line.startsWith('data:'))) notify(false);
              }
            }
          } finally { reader.releaseLock(); }
        } catch {
          if (!controller.signal.aborted) {
            if (window.__tonkCatalogStream === state) delete window.__tonkCatalogStream;
            notify(true);
          }
        }
      })();
    }
    """
}
