import {TonkServer, ServerOptions} from '../index';
import fs from 'fs';
import path from 'path';
import os from 'os';
import supertest from 'supertest';
import AdmZip from 'adm-zip';
import rimraf from 'rimraf';
import {v4 as uuidv4} from 'uuid';

describe('TonkServer', () => {
  let server: TonkServer;
  let request: supertest.SuperTest<supertest.Test>;
  let tempDir: string;
  let bundlesDir: string;

  beforeEach(async () => {
    // Create temp directories for testing
    tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'tonk-test-'));
    bundlesDir = path.join(tempDir, 'bundles');

    const options: ServerOptions = {
      bundlesPath: bundlesDir,
      dirPath: path.join(tempDir, 'stores'),
      verbose: false,
    };

    server = new TonkServer(options);
    await server.start();

    // Get the express app for testing with supertest
    // @ts-ignore - accessing private property for testing
    request = supertest(server.app);
  });

  afterEach(async () => {
    await server.stop();
    // Clean up test directories
    rimraf.sync(tempDir);
  });

  describe('Health check', () => {
    it('should respond with pong', async () => {
      const response = await request.get('/ping');
      expect(response.status).toBe(200);
      expect(response.text).toBe('pong');
    });
  });

  describe('upload-bundle endpoint', () => {
    let testZipPath: string;

    beforeEach(() => {
      // Create a test zip file
      const zip = new AdmZip();
      zip.addFile(
        'index.html',
        Buffer.from('<html><body>Test bundle</body></html>'),
      );
      zip.addFile('app.js', Buffer.from('console.log("Test bundle");'));
      testZipPath = path.join(tempDir, 'test-bundle.zip');
      zip.writeZip(testZipPath);
    });

    it('should upload and extract a bundle', async () => {
      const response = await request
        .post('/upload-bundle')
        .attach('bundle', testZipPath)
        .field('name', 'test-bundle');

      expect(response.status).toBe(200);
      expect(response.body.success).toBe(true);
      expect(response.body.bundleName).toBe('test-bundle');

      // Check if files were extracted
      expect(fs.existsSync(path.join(bundlesDir, 'test-bundle'))).toBe(true);
      expect(
        fs.existsSync(path.join(bundlesDir, 'test-bundle', 'index.html')),
      ).toBe(true);
      expect(
        fs.existsSync(path.join(bundlesDir, 'test-bundle', 'app.js')),
      ).toBe(true);
    });

    it('should return 400 if no bundle is uploaded', async () => {
      const response = await request.post('/upload-bundle');

      expect(response.status).toBe(400);
      expect(response.body.error).toBe('No bundle file uploaded');
    });

    it('should use the file name if no name is provided', async () => {
      const response = await request
        .post('/upload-bundle')
        .attach('bundle', testZipPath);

      expect(response.status).toBe(200);
      expect(response.body.success).toBe(true);
      expect(response.body.bundleName).toBe('test-bundle');
    });

    it('should handle race conditions with same bundle name', async () => {
      // Create a second test zip with different content
      const zip2 = new AdmZip();
      zip2.addFile(
        'index.html',
        Buffer.from('<html><body>Second bundle</body></html>'),
      );
      zip2.addFile('app.js', Buffer.from('console.log("Second bundle");'));
      const testZipPath2 = path.join(tempDir, 'test-bundle-2.zip');
      zip2.writeZip(testZipPath2);

      // Send both requests almost simultaneously
      const promise1 = request
        .post('/upload-bundle')
        .attach('bundle', testZipPath)
        .field('name', 'same-name-bundle');

      const promise2 = request
        .post('/upload-bundle')
        .attach('bundle', testZipPath2)
        .field('name', 'same-name-bundle');

      // Wait for both requests to complete
      const [response1, response2] = await Promise.all([promise1, promise2]);

      // Both should succeed
      expect(response1.status).toBe(200);
      expect(response2.status).toBe(200);

      // Read the content of the index.html file to determine which upload "won"
      const finalContent = fs.readFileSync(
        path.join(bundlesDir, 'same-name-bundle', 'index.html'),
        'utf8',
      );

      // One of the two contents should be present, based on which request finished last
      const isContent1 = finalContent.includes('Test bundle');
      const isContent2 = finalContent.includes('Second bundle');

      expect(isContent1 || isContent2).toBe(true);
      // But not both - it should have been overwritten by one of them
      expect(isContent1 && isContent2).toBe(false);
    });

    it('should sequentially process uploads with unique identifiers to prevent conflicts', async () => {
      // This test demonstrates a potential fix for the race condition

      // Create unique bundle directories using UUIDs
      const testUploads = Array(5)
        .fill(null)
        .map(() => {
          const uid = uuidv4().substring(0, 8);
          const zip = new AdmZip();
          zip.addFile(
            'index.html',
            Buffer.from(`<html><body>Bundle ${uid}</body></html>`),
          );
          zip.addFile('app.js', Buffer.from(`console.log("Bundle ${uid}");`));
          const zipPath = path.join(tempDir, `bundle-${uid}.zip`);
          zip.writeZip(zipPath);

          return {
            uid,
            zipPath,
          };
        });

      // Upload all bundles simultaneously
      const promises = testUploads.map(({zipPath}) =>
        request
          .post('/upload-bundle')
          .attach('bundle', zipPath)
          // Use a shared bundle name but with the UUID to avoid conflicts
          .field('name', `shared-bundle-${uuidv4().substring(0, 8)}`),
      );

      const responses = await Promise.all(promises);

      // All uploads should succeed
      responses.forEach(response => {
        expect(response.status).toBe(200);
        expect(response.body.success).toBe(true);
      });

      // We should have unique bundle directories
      const bundleCount = fs.readdirSync(bundlesDir).length;
      expect(bundleCount).toBe(testUploads.length);
    });
  });

  describe('bundle server management', () => {
    let testBundlePath: string;

    beforeEach(async () => {
      // Create a test bundle
      testBundlePath = path.join(bundlesDir, 'test-server-bundle');
      fs.mkdirSync(testBundlePath, {recursive: true});
      fs.writeFileSync(
        path.join(testBundlePath, 'index.html'),
        '<html><body>Test server</body></html>',
      );
    });

    it('should start a bundle server', async () => {
      const response = await request
        .post('/start')
        .send({bundleName: 'test-server-bundle'});

      expect(response.status).toBe(200);
      expect(response.body.bundleName).toBe('test-server-bundle');
      expect(response.body.status).toBe('running');
      expect(response.body.id).toBeTruthy();
      expect(typeof response.body.port).toBe('number');
    });

    it('should return 404 for non-existent bundle', async () => {
      const response = await request
        .post('/start')
        .send({bundleName: 'non-existent-bundle'});

      expect(response.status).toBe(404);
    });

    it('should return 400 if no bundle name is provided', async () => {
      const response = await request.post('/start').send({});

      expect(response.status).toBe(400);
      expect(response.body.error).toBe('Bundle name is required');
    });

    it('should kill a running bundle server', async () => {
      // First start a server
      const startResponse = await request
        .post('/start')
        .send({bundleName: 'test-server-bundle'});

      const serverId = startResponse.body.id;

      // Then stop it
      const killResponse = await request.post('/kill').send({id: serverId});

      expect(killResponse.status).toBe(200);
      expect(killResponse.body.success).toBe(true);
    });

    it('should return 404 when killing non-existent server', async () => {
      const response = await request
        .post('/kill')
        .send({id: 'non-existent-id'});

      expect(response.status).toBe(404);
    });

    it('should list running servers', async () => {
      // Start a server first
      const startResponse = await request
        .post('/start')
        .send({bundleName: 'test-server-bundle'});

      const serverId = startResponse.body.id;

      // Get the list of servers
      const psResponse = await request.get('/ps');

      expect(psResponse.status).toBe(200);
      expect(Array.isArray(psResponse.body)).toBe(true);

      // Find our server in the list
      const server = psResponse.body.find((s: any) => s.id === serverId);
      expect(server).toBeTruthy();
      expect(server.bundleName).toBe('test-server-bundle');
      expect(server.status).toBe('running');
    });
  });
});
