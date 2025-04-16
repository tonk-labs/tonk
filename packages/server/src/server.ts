import fs from 'fs';
import express from 'express';
import {WebSocketServer} from 'ws';
import {Repo} from '@automerge/automerge-repo';
import {NodeWSServerAdapter} from '@automerge/automerge-repo-network-websocket';
import {NodeFSStorageAdapter} from '@automerge/automerge-repo-storage-nodefs';
import os from 'os';

export class Server {
  private socket;
  private server;
  private readyResolvers: ((value: any) => void)[] = [];
  private isReady = false;
  // @ts-ignore: Used by the repo configuration
  private repo: Repo;

  constructor() {
    const dir =
      process.env.DATA_DIR !== undefined ? process.env.DATA_DIR : '.amrg';
    if (!fs.existsSync(dir)) {
      fs.mkdirSync(dir);
    }

    var hostname = os.hostname();

    this.socket = new WebSocketServer({noServer: true});

    const PORT =
      process.env.PORT !== undefined ? parseInt(process.env.PORT) : 3030;
    const app = express();
    app.use(express.static('public'));

    const config = {
      network: [new NodeWSServerAdapter(this.socket as any)],
      storage: new NodeFSStorageAdapter(dir),
      /** @ts-ignore @type {(import("@automerge/automerge-repo").PeerId)}  */
      peerId: `storage-server-${hostname}`,
      // Since this is a server, we don't share generously — meaning we only sync documents they already
      // know about and can ask for by ID.
      sharePolicy: async () => false,
    };
    this.repo = new Repo(config as any);

    this.server = app.listen(PORT, () => {
      console.log(`Listening on port ${PORT}`);
      this.isReady = true;
      this.readyResolvers.forEach(resolve => resolve(true));
    });

    this.server.on('upgrade', (request, socket, head) => {
      this.socket.handleUpgrade(request, socket, head, socket => {
        this.socket.emit('connection', socket, request);
      });
    });
  }

  async ready() {
    if (this.isReady) {
      return true;
    }

    return new Promise(resolve => {
      this.readyResolvers.push(resolve);
    });
  }

  close() {
    this.socket.close();
    this.server.close();
  }
}
