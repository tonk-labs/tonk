import { serveStdio } from '@modelcontextprotocol/server/stdio';
import { connectDesktop } from './desktop.mjs';
import { createTonkServer } from './core.mjs';

export async function main(args = process.argv.slice(2)) {
  if (args.length !== 1) {
    console.error('Usage: node server.mjs /absolute/path/to/MCP/connection.json');
    process.exitCode = 1;
    return;
  }
  try {
    const backend = await connectDesktop(args[0]);
    serveStdio(() => createTonkServer(backend), {
      onerror: () => console.error('MCP transport error.'),
    });
  } catch {
    console.error('Could not connect to the local Tonk runtime. Check that the app is running with --mcp-space and that its private connection file is available.');
    process.exitCode = 1;
  }
}

await main();
