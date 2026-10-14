import Foundation
import HarnessCore

@MainActor
extension RuntimeModel {
  /// The same scoped handoff command used by Tonk's "connect a tool" surface.
  /// The private link stays inside the native adapter and never reaches the model.
  func createCLILink(_ space: TonkSpace) async throws -> String {
    try requireSpaceReady(space)
    let response = try await accountScript(
      """
      const path = '/api/repository/' + encodeURIComponent(subject) + '/branch/main';
      const field = (the, as) => ({the, as, cardinality: 'one'});
      const variable = name => ({'?': {name}});
      const query = {predicate: {with: {
        status: field('xyz.tonk.agent-handoff/status', 'Text'),
        link: field('xyz.tonk.agent-handoff/link', 'Text'),
        mode: {...field('xyz.tonk.agent-handoff/mode', 'Text'), optional: true}
      }}, terms: {this: subject, status: variable('status'), link: variable('link'), mode: variable('mode')}};
      await api(path + '/transact', {claims: [{op: 'assert', application: {
        predicate: {kind: 'transient', concept: {with: {
          time: field('xyz.tonk.agent-handoff/time', 'Float'),
          fresh: field('xyz.tonk.agent-handoff/fresh', 'Text'),
          space: field('xyz.tonk.agent-handoff/space', 'Entity')
        }}}, parameters: {time: Date.now(), fresh: 'new', space: subject}
      }}]});
      const controller = new AbortController();
      const timeout = setTimeout(() => controller.abort(), 30000);
      let reader;
      try {
        const response = await fetch(path + '/query', {method: 'POST',
          headers: {'Content-Type': 'application/json', Accept: 'text/event-stream'},
          body: JSON.stringify(query), signal: controller.signal});
        if (!response.ok) throw new Error('Invitation subscription failed (HTTP ' + response.status + ').');
        reader = response.body.getReader();
        const decoder = new TextDecoder();
        let buffer = '';
        let lastMode = 'no response';
        while (true) {
          const chunk = await reader.read();
          if (chunk.done) throw new Error('Invitation subscription ended (mode: ' + lastMode + ').');
          buffer += decoder.decode(chunk.value, {stream: true});
          if (buffer.length > 1000000) throw new Error('Invitation response is too large.');
          let end;
          while ((end = buffer.indexOf('\\n\\n')) >= 0) {
            const frame = buffer.slice(0, end); buffer = buffer.slice(end + 2);
            const data = frame.split('\\n').filter(line => line.startsWith('data:')).map(line => line.slice(5).trim()).join('\\n');
            if (!data) continue;
            const payload = JSON.parse(data);
            const rows = Array.isArray(payload) ? payload : (payload.conclusions || payload.asserted || []);
            for (const row of rows) {
              const result = row.fields;
              lastMode = result?.mode || 'missing mode';
              if (result?.status === 'ready' && result.mode === 'scoped' && result.link)
                return {link: result.link};
              if (result?.status === 'ready' && !result.mode)
                throw new Error('The runtime returned an invitation without scoped-tool mode, including on its live subscription.');
            }
          }
        }
      } finally {
        clearTimeout(timeout); controller.abort(); if (reader) await reader.cancel().catch(() => {});
      }
      """, arguments: ["subject": space.subject])
    guard let link = response["link"] as? String, let url = URL(string: link),
      RuntimeLocation.isEmbedded(url)
    else { throw HarnessError.message("Tonk returned an unsupported CLI link.") }
    return link
  }
}
