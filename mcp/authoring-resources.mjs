import { guides, instructions } from './authoring.mjs';

export function registerAuthoringResources(server) {
  for (const [topic, text] of Object.entries({ context: instructions, ...guides })) {
    const uri = 'tonk://guide/' + topic;
    server.registerResource(
      'tonk-guide-' + topic,
      uri,
      { mimeType: 'text/markdown' },
      async () => ({
        contents: [{ uri, mimeType: 'text/markdown', text }],
      }),
    );
  }
}
