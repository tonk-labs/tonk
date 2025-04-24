import {TonkServer, ServerOptions} from '../index.js';
import fs from 'fs';
import path from 'path';
import os from 'os';
import supertest from 'supertest';
import * as tar from 'tar';
import rimraf from 'rimraf';
import {v4 as uuidv4} from 'uuid';

describe('TonkServer', () => {
  let server: TonkServer;
  let request: supertest.SuperTest<supertest.Test>;
  let tempDir: string;
  let bundlesDir: string;
  let testBundleDir: string;

  beforeEach(async () => {
    // Create temp directories for testing
    tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'tonk-test-'));
    bundlesDir = path.join(tempDir, 'bundles');
    testBundleDir = path.join(tempDir, 'test-bundle-files');

    // Create directory for test files
    fs.mkdirSync(testBundleDir, {recursive: true});

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
    let testTarPath: string;

    beforeEach(async () => {
      // Create test files
      fs.writeFileSync(
        path.join(testBundleDir, 'index.html'),
        '<html><body>Test bundle</body></html>',
      );
      fs.writeFileSync(
        path.join(testBundleDir, 'app.js'),
        'console.log("Test bundle");',
      );

      // Create a test tar.gz file
      testTarPath = path.join(tempDir, 'test-bundle.tar.gz');
      await tar.create(
        {
          gzip: true,
          file: testTarPath,
          cwd: tempDir,
        },
        ['test-bundle-files'],
      );
    });

    it('should upload and extract a bundle', async () => {
      const response = await request
        .post('/upload-bundle')
        .attach('bundle', testTarPath)
        .field('name', 'test-bundle');

      expect(response.status).toBe(200);
      expect(response.body.success).toBe(true);
      expect(response.body.bundleName).toBe('test-bundle');

      // Check if files were extracted
      expect(fs.existsSync(path.join(bundlesDir, 'test-bundle'))).toBe(true);
      expect(
        fs.existsSync(
          path.join(
            bundlesDir,
            'test-bundle',
            'test-bundle-files',
            'index.html',
          ),
        ),
      ).toBe(true);
      expect(
        fs.existsSync(
          path.join(bundlesDir, 'test-bundle', 'test-bundle-files', 'app.js'),
        ),
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
        .attach('bundle', testTarPath);

      expect(response.status).toBe(200);
      expect(response.body.success).toBe(true);
      expect(response.body.bundleName).toBe('test-bundle');
    });

    it('should handle race conditions with same bundle name', async () => {
      // Create second test directory
      const testBundleDir2 = path.join(tempDir, 'test-bundle-files-2');
      fs.mkdirSync(testBundleDir2, {recursive: true});

      // Create second test files
      fs.writeFileSync(
        path.join(testBundleDir2, 'index.html'),
        '<html><body>Second bundle</body></html>',
      );
      fs.writeFileSync(
        path.join(testBundleDir2, 'app.js'),
        'console.log("Second bundle");',
      );

      // Create a second test tar.gz file
      const testTarPath2 = path.join(tempDir, 'test-bundle-2.tar.gz');
      await tar.create(
        {
          gzip: true,
          file: testTarPath2,
          cwd: tempDir,
        },
        ['test-bundle-files-2'],
      );

      // Send both requests almost simultaneously
      const promise1 = request
        .post('/upload-bundle')
        .attach('bundle', testTarPath)
        .field('name', 'same-name-bundle');

      const promise2 = request
        .post('/upload-bundle')
        .attach('bundle', testTarPath2)
        .field('name', 'same-name-bundle');

      // Wait for both requests to complete
      const [response1, response2] = await Promise.all([promise1, promise2]);

      // Both should succeed
      expect(response1.status).toBe(200);
      expect(response2.status).toBe(200);

      // The extracted content structure will be different with tar.gz, so let's
      // check if both extracted directories exist - one of them should have "won"
      const extractedDir = path.join(bundlesDir, 'same-name-bundle');
      const hasFiles1 = fs.existsSync(
        path.join(extractedDir, 'test-bundle-files'),
      );
      const hasFiles2 = fs.existsSync(
        path.join(extractedDir, 'test-bundle-files-2'),
      );

      // One of the directories should exist
      expect(hasFiles1 || hasFiles2).toBe(true);
    });

    it('should sequentially process uploads with unique identifiers to prevent conflicts', async () => {
      // Create unique bundle directories using UUIDs
      const testUploads = await Promise.all(
        Array(5)
          .fill(null)
          .map(async () => {
            const uid = uuidv4().substring(0, 8);

            // Create a unique test directory
            const testDir = path.join(tempDir, `test-dir-${uid}`);
            fs.mkdirSync(testDir, {recursive: true});

            // Create test files
            fs.writeFileSync(
              path.join(testDir, 'index.html'),
              `<html><body>Bundle ${uid}</body></html>`,
            );
            fs.writeFileSync(
              path.join(testDir, 'app.js'),
              `console.log("Bundle ${uid}");`,
            );

            // Create a tar.gz archive
            const tarPath = path.join(tempDir, `bundle-${uid}.tar.gz`);
            await tar.create(
              {
                gzip: true,
                file: tarPath,
                cwd: tempDir,
              },
              [`test-dir-${uid}`],
            );

            return {
              uid,
              tarPath,
            };
          }),
      );

      // Upload all bundles simultaneously
      const promises = testUploads.map(({tarPath}) =>
        request
          .post('/upload-bundle')
          .attach('bundle', tarPath)
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
