import { Repo, type RepoConfig } from '@automerge/automerge-repo';
import { NodeWSServerAdapter } from '@automerge/automerge-repo-network-websocket';
import cors from 'cors';
import express from 'express';
import { readFileSync } from 'fs';
import { dirname, join } from 'path';
import { fileURLToPath } from 'url';
import { WebSocketServer } from 'ws';
import { BundleStorageAdapter } from './bundleStorageAdapter.js';
import { S3Storage } from './s3-storage.js';
import JSZip from 'jszip';

class Server {
  #socket: WebSocketServer;

  #server: ReturnType<import('express').Express['listen']>;
  #storage: BundleStorageAdapter;

  #repo: Repo;
  #rootDocumentId: string | null = null;

  static async create(port: number, bundlePath: string): Promise<Server> {
    // Read bundle file as bytes
    const bundleBytes = readFileSync(bundlePath);

    // Create storage adapter from bundle
    const storage = await BundleStorageAdapter.fromBundle(bundleBytes);

    return new Server(port, storage);
  }

  private constructor(port: number, storage: BundleStorageAdapter) {
    this.#socket = new WebSocketServer({ noServer: true });

    this.#socket.on('error', error => {
      console.error('WebSocket server error:', error);
    });

    const PORT = port;
    const app = express();

    // Enable CORS for all routes to allow browser clients to fetch root document

    app.use(
      cors({
        origin: true, // Allow all origins
        credentials: true, // Allow cookies/credentials
        methods: ['GET', 'POST', 'PUT', 'DELETE', 'OPTIONS'],
        allowedHeaders: ['Content-Type', 'Authorization'],
      })
    );

    app.use(express.static('public'));
    app.use(express.raw({ type: 'application/octet-stream', limit: '100mb' }));
    app.use(express.json());
    this.#storage = storage;

    const config: RepoConfig = {
      network: [new NodeWSServerAdapter(this.#socket as any) as any],
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

    app.get('/tonk_core_bg.wasm', (_req, res) => {
      try {
        // Read the WASM file from the server directory
        const __filename = fileURLToPath(import.meta.url);
        const __dirname = dirname(__filename);
        const wasmPath = join(
          __dirname,
          '..',
          '..',
          '..',
          'packages',
          'core-js',
          'dist',
          'tonk_core_bg.wasm'
        );
        const wasmBuffer = readFileSync(wasmPath);

        // Set headers similar to esm.sh
        res.setHeader('Content-Type', 'application/wasm');
        res.setHeader('Cache-Control', 'public, max-age=31536000, immutable');
        res.setHeader('Access-Control-Allow-Origin', '*');
        res.setHeader('Access-Control-Allow-Methods', 'GET, HEAD, OPTIONS');
        res.setHeader('Content-Length', wasmBuffer.length.toString());

        // Send the WASM file
        res.send(wasmBuffer);
      } catch (error) {
        console.error('Error serving WASM file:', error);
        res.status(404).json({ error: 'WASM file not found' });
      }
    });

    // Endpoint to get the manifest as a slim bundle (zip file with just manifest.json)
    app.get('/.manifest.tonk', async (_req, res) => {
      console.log('Received request for /.manifest.tonk');
      try {
        console.log('Creating slim bundle...');
        const slimBundle = await this.#storage.createSlimBundle();
        console.log(
          'Slim bundle created successfully, size:',
          slimBundle.length
        );

        // Set appropriate headers for zip file download
        res.setHeader('Content-Type', 'application/zip');
        res.setHeader(
          'Content-Disposition',
          'attachment; filename="manifest.tonk"'
        );
        res.setHeader('Content-Length', slimBundle.length.toString());

        res.send(Buffer.from(slimBundle));
        console.log('Slim bundle sent successfully');
      } catch (error) {
        console.error('Error creating slim bundle:', error);
        res.status(500).json({ error: 'Failed to create manifest bundle' });
      }
    });

    // Initialize S3 storage if configured
    const s3Storage = new S3Storage({
      bucket: 'host-web-bundle-storage',
      region: 'eu-north-1',
    });

    // API endpoint to upload a bundle
    app.post('/api/bundles', async (req, res) => {
      if (!s3Storage) {
        return res.status(503).json({ error: 'S3 storage not configured' });
      }

      try {
        const bundleData = req.body;

        if (!Buffer.isBuffer(bundleData) || bundleData.length === 0) {
          return res.status(400).json({ error: 'Invalid bundle data' });
        }

        // Extract bundle ID from manifest
        const zip = new JSZip();
        const zipContent = await zip.loadAsync(bundleData);
        const manifestFile = zipContent.file('manifest.json');

        if (!manifestFile) {
          return res
            .status(400)
            .json({ error: 'Invalid bundle: manifest.json not found' });
        }

        const manifestContent = await manifestFile.async('text');
        const manifest = JSON.parse(manifestContent);
        const bundleId = manifest.rootId;

        if (!bundleId) {
          return res
            .status(400)
            .json({ error: 'Invalid bundle: rootId not found in manifest' });
        }

        // Upload to S3
        await s3Storage.uploadBundle(bundleId, bundleData);

        res.json({
          id: bundleId,
          message: 'Bundle uploaded successfully',
        });
      } catch (error) {
        console.error('Error uploading bundle:', error);
        res.status(500).json({ error: 'Failed to upload bundle' });
      }
    });

    // API endpoint to download a bundle manifest (slim bundle)
    app.get('/api/bundles/:id/manifest', async (req, res) => {
      if (!s3Storage) {
        return res.status(503).json({ error: 'S3 storage not configured' });
      }

      try {
        const bundleId = req.params.id;

        // Download from S3
        const bundleData = await s3Storage.downloadBundle(bundleId);

        // Return slim bundle (just manifest + root document storage)
        const zip = new JSZip();
        const zipContent = await zip.loadAsync(bundleData!);

        // Create new slim bundle
        const slimZip = new JSZip();

        // Add manifest
        const manifestFile = zipContent.file('manifest.json');
        if (manifestFile) {
          const manifestContent = await manifestFile.async('uint8array');
          slimZip.file('manifest.json', manifestContent);
        }

        // Add storage files for the root document
        const rootIdPrefix = bundleId.substring(0, 2);
        const storageFolderPrefix = `storage/${rootIdPrefix}`;

        for (const [relativePath, file] of Object.entries(zipContent.files)) {
          const zipFile = file as any;
          if (
            !zipFile.dir &&
            relativePath !== 'manifest.json' &&
            relativePath.startsWith(storageFolderPrefix)
          ) {
            const content = await zipFile.async('uint8array');
            slimZip.file(relativePath, content);
          }
        }

        // Generate slim bundle
        const slimBundleData = await slimZip.generateAsync({
          type: 'uint8array',
          compression: 'DEFLATE',
          compressionOptions: { level: 6 },
        });

        res.setHeader('Content-Type', 'application/zip');
        res.setHeader(
          'Content-Disposition',
          `attachment; filename="${bundleId}.tonk"`
        );
        res.setHeader('Content-Length', slimBundleData.length.toString());
        res.setHeader('Cache-Control', 'public, max-age=3600');
        res.send(Buffer.from(slimBundleData));
      } catch (error: any) {
        console.error('Error downloading bundle:', error);
        if (error.message?.includes('not found')) {
          res.status(404).json({ error: 'Bundle not found' });
        } else {
          res.status(500).json({ error: 'Failed to download bundle' });
        }
      }
    });

    // API endpoint to download full bundle
    app.get('/api/bundles/:id', async (req, res) => {
      if (!s3Storage) {
        return res.status(503).json({ error: 'S3 storage not configured' });
      }

      try {
        const bundleId = req.params.id;

        // Download from S3
        const bundleData = await s3Storage.downloadBundle(bundleId);

        res.setHeader('Content-Type', 'application/octet-stream');
        res.setHeader(
          'Content-Disposition',
          `attachment; filename="${bundleId}.tonk"`
        );
        res.setHeader('Content-Length', bundleData!.length.toString());
        res.setHeader('Cache-Control', 'public, max-age=3600');
        res.send(bundleData);
      } catch (error: any) {
        console.error('Error downloading bundle:', error);
        if (error.message?.includes('not found')) {
          res.status(404).json({ error: 'Bundle not found' });
        } else {
          res.status(500).json({ error: 'Failed to download bundle' });
        }
      }
    });

    // API endpoint to get blank tonk template
    app.get('/api/blank-tonk', async (_req, res) => {
      try {
        const __filename = fileURLToPath(import.meta.url);
        const __dirname = dirname(__filename);
        const blankTonkPath = join(__dirname, '..', 'latergram.tonk');
        const blankTonkBuffer = readFileSync(blankTonkPath);

        res.setHeader('Content-Type', 'application/octet-stream');
        res.setHeader(
          'Content-Disposition',
          'attachment; filename="latergram.tonk"'
        );
        res.setHeader('Content-Length', blankTonkBuffer.length.toString());
        res.setHeader('Cache-Control', 'public, max-age=31536000');
        res.send(blankTonkBuffer);
      } catch (error) {
        console.error('Error serving blank tonk:', error);
        res.status(404).json({ error: 'Blank tonk template not found' });
      }
    });

    this.#server = app.listen(PORT, () => {
      const address = this.#server.address();
      console.log(
        `Listening on port ${typeof address === 'string' ? address : address?.port}`
      );
    });

    this.#server.on('upgrade', (request, socket, head) => {
      console.log('upgrading to websocket');
      this.#socket.handleUpgrade(request, socket, head, ws => {
        this.#socket.emit('connection', ws, request);
      });
    });

    this.#server.on('error', error => {
      console.error('HTTP server error:', error);
      console.error('Process will exit and systemd will restart it');
      process.exit(1);
    });

    this.#socket.on('error', error => {
      console.error('WebSocket server error:', error);
    });
  }

  close() {
    this.#storage.log();
    this.#socket.close();
    this.#server.close();
  }
}

async function main() {
  const port = process.argv[2] ? parseInt(process.argv[2]) : 8080;
  const bundlePath = process.argv[3];

  if (!bundlePath) {
    console.error('Error: Bundle path is required');
    console.error('Usage: node server.js [port] <bundle-path>');
    process.exit(1);
  }

  try {
    const server = await Server.create(port, bundlePath);

    process.on('SIGINT', () => {
      server.close();
      process.exit(0);
    });
  } catch (error) {
    console.error('Failed to start server:', error);
    process.exit(1);
  }
}

process.on('uncaughtException', error => {
  console.error('Uncaught exception:', error);
  console.error('Process will exit and systemd will restart it');
  process.exit(1);
});

process.on('unhandledRejection', (reason, promise) => {
  console.error('Unhandled rejection at:', promise, 'reason:', reason);
  console.error('Process will exit and systemd will restart it');
  process.exit(1);
});

main().catch(error => {
  console.error('Fatal error in main:', error);
  process.exit(1);
});
