import {
  type Chunk,
  Repo,
  type RepoConfig,
  type StorageAdapterInterface,
  type StorageKey,
} from '@automerge/automerge-repo';
import { NodeWSServerAdapter } from '@automerge/automerge-repo-network-websocket';
import cors from 'cors';
import express from 'express';
import { WebSocketServer } from 'ws';

class Server {
  #socket: WebSocketServer;

  #server: ReturnType<import('express').Express['listen']>;
  #storage: InMemoryStorageAdapter;

  #repo: Repo;
  #rootDocumentId: string | null = null;

  constructor(port: number) {
    this.#socket = new WebSocketServer({ noServer: true });

    const PORT = port;
    const app = express();

    // Enable CORS for all routes to allow browser clients to fetch root document
    app.use(cors());

    app.use(express.static('public'));
    this.#storage = new InMemoryStorageAdapter();

    const config: RepoConfig = {
      // network: [new NodeWSServerAdapter(this.#socket) as any],
      network: [new NodeWSServerAdapter(this.#socket as any)],
      storage: this.#storage,
      /** @ts-expect-error @type {(import("automerge-repo").PeerId)}  */
      peerId: `storage-server` as PeerId,
      // Since this is a server, we don't share generously — meaning we only sync documents they already
      // know about and can ask for by ID.
      sharePolicy: async () => true,
    };
    const serverRepo = new Repo(config);
    this.#repo = serverRepo;

    app.get('/', (_req, res) => {
      res.send(`👍 @automerge/automerge-repo-sync-server is running`);
    });

    // Endpoint to get the canonical root filesystem document ID
    app.get('/root', (_req, res) => {
      const rootId = this.getOrCreateRootDocument();
      res.json({ rootDocumentId: rootId });
    });

    this.#server = app.listen(PORT, () => {
      const address = this.#server.address();
      console.log(
        `Listening on port ${typeof address === 'string' ? address : address?.port}`
      );
    });

    this.#server.on('upgrade', (request, socket, head) => {
      console.log('upgrading to websocket');
      this.#socket.handleUpgrade(request, socket, head, socket => {
        this.#socket.emit('connection', socket, request);
      });
    });
  }

  getOrCreateRootDocument(): string {
    if (this.#rootDocumentId) {
      return this.#rootDocumentId;
    }

    // Create a canonical root filesystem document
    // Use a deterministic approach to ensure consistency
    const rootDoc = this.#repo.create();

    // Initialize it as a directory structure (similar to VFS root)
    rootDoc.change((doc: any) => {
      doc.type = 'directory';
      doc.name = '/';
      doc.children = {};
      doc.created_at = new Date().toISOString();
      doc.modified_at = new Date().toISOString();
    });

    this.#rootDocumentId = rootDoc.documentId;
    console.log(
      `Created canonical root filesystem document: ${this.#rootDocumentId}`
    );

    return this.#rootDocumentId;
  }

  close() {
    this.#storage.log();
    this.#socket.close();
    this.#server.close();
  }
}

class InMemoryStorageAdapter implements StorageAdapterInterface {
  #data: Map<StorageKey, Uint8Array> = new Map();

  async load(key: StorageKey): Promise<Uint8Array | undefined> {
    return this.#data.get(key);
  }
  async save(key: StorageKey, data: Uint8Array): Promise<void> {
    this.#data.set(key, data);
  }
  async remove(key: StorageKey): Promise<void> {
    this.#data.delete(key);
  }
  async loadRange(keyPrefix: StorageKey): Promise<Chunk[]> {
    const result: Chunk[] = [];
    for (const [key, value] of this.#data.entries()) {
      if (isPrefixOf(keyPrefix, key)) {
        result.push({
          key,
          data: value,
        });
      }
    }
    return result;
  }

  removeRange(keyPrefix: StorageKey): Promise<void> {
    for (const [key] of this.#data.entries()) {
      if (isPrefixOf(keyPrefix, key)) {
        this.#data.delete(key);
      }
    }
    return Promise.resolve();
  }

  log() {
    console.log(`InMemoryStorageAdapter has ${this.#data.size} items:`);
    for (const [key, value] of this.#data.entries()) {
      console.log(`  ${key.join('/')}: ${value.length} bytes`);
    }
  }
}

function isPrefixOf(prefix: StorageKey, candidate: StorageKey): boolean {
  return (
    prefix.length <= candidate.length &&
    prefix.every((segment, index) => segment === candidate[index])
  );
}

const port = process.argv[2] ? parseInt(process.argv[2]) : 8081;
const server = new Server(port);

process.on('SIGINT', () => {
  server.close();
  process.exit(0);
});
