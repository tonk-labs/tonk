import { fileURLToPath, pathToFileURL } from 'node:url';
import { resolve } from 'node:path';

// All desktop transports use the MCP package in this same monorepo checkout.
const root = fileURLToPath(new URL('../../mcp/', import.meta.url));
export const sharedModule = name => pathToFileURL(resolve(root, name)).href;
