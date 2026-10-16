// Stdio MCP server for a persistent local Tonk peer.
//
//   node scripts/local-peer.mjs BINARY [DATA_DIRECTORY] [TRUSTED_REMOTE] [LINK_ORIGIN]
//
// DATA_DIRECTORY defaults to TONK_PEER_HOME, else ~/.tonk/peer. Every host on
// this machine that uses the same directory acts as the same peer.
// TRUSTED_REMOTE defaults to TONK_PEER_REMOTE, else the dev service.
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';
import { StdioServerTransport } from '@modelcontextprotocol/server/stdio';
import { createTonkServer } from '../core.mjs';
import { localPeerBackend } from '../local-peer-backend.mjs';

const [binary, directory, remoteArgument, linkOrigin] = process.argv.slice(2);
if (!binary) {
  throw Error(
    'Usage: node scripts/local-peer.mjs BINARY [DATA_DIRECTORY] [TRUSTED_REMOTE] [LINK_ORIGIN]',
  );
}
const dataDirectory = resolve(
  directory || process.env.TONK_PEER_HOME || join(homedir(), '.tonk', 'peer'),
);
const remote = remoteArgument || process.env.TONK_PEER_REMOTE || 'https://tonk.foundation/ucan/';
const backend = localPeerBackend({ binary: resolve(binary), dataDirectory, remote, linkOrigin });
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
