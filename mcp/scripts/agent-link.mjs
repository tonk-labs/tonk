import { StdioServerTransport } from '@modelcontextprotocol/server/stdio';
import { createTonkServer } from '../core.mjs';
import { agentLinkBackend } from '../agent-link-backend.mjs';

const [binary, dataDirectory, remote, linkOrigin] = process.argv.slice(2);
if (!binary || !dataDirectory || !remote) {
  throw Error(
    'Usage: node scripts/agent-link.mjs BINARY PRIVATE_DATA_DIRECTORY TRUSTED_REMOTE [LINK_ORIGIN]',
  );
}
const backend = agentLinkBackend({ binary, dataDirectory, remote, linkOrigin });
const server = createTonkServer(backend);
server.onclose = () => {
  void backend.close({ cancel: true });
};
const close = async () => {
  await backend.close({ cancel: true });
  await server.close();
};
process.once('SIGTERM', close);
process.once('SIGINT', close);
await server.connect(new StdioServerTransport());
