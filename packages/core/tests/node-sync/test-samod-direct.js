#!/usr/bin/env node

import { create_sync_engine } from '../../pkg-node/tonk_core.js';
import WebSocket from 'ws';

class SamodTestClient {
  constructor(clientId, serverUrl = 'ws://127.0.0.1:8080') {
    this.clientId = clientId;
    this.serverUrl = serverUrl;
    this.syncEngine = null;
    this.repo = null;
    this.connected = false;
  }

  async init() {
    try {
      console.log(`[${this.clientId}] Creating SyncEngine...`);
      this.syncEngine = await create_sync_engine();

      // Get the samod repo directly instead of VFS
      console.log(`[${this.clientId}] Getting Repo from SyncEngine...`);
      this.repo = await this.syncEngine.getRepo();
      console.log(`[${this.clientId}] Repo obtained successfully`);

      console.log(`[${this.clientId}] SyncEngine created successfully`);
      const peerId = await this.syncEngine.getPeerId();
      console.log(`[${this.clientId}] Peer ID: ${peerId}`);

      return true;
    } catch (error) {
      console.error(`[${this.clientId}] Failed to create SyncEngine:`, error);
      return false;
    }
  }

  async connect() {
    if (!this.syncEngine) {
      throw new Error(`[${this.clientId}] Must call init() first`);
    }

    try {
      console.log(
        `[${this.clientId}] Connecting to WebSocket: ${this.serverUrl}`
      );
      await this.syncEngine.connectWebsocket(this.serverUrl);
      console.log(`[${this.clientId}] Connected to WebSocket successfully`);
      this.connected = true;

      // Wait a moment for connection to stabilize
      await new Promise(resolve => setTimeout(resolve, 500));
      return true;
    } catch (error) {
      console.error(
        `[${this.clientId}] Failed to connect to WebSocket:`,
        error
      );
      return false;
    }
  }

  async createDocument(
    docId = null,
    content = `Hello from ${this.clientId} at ${Date.now()}!`
  ) {
    if (!this.repo) {
      throw new Error(`[${this.clientId}] No Repo available`);
    }

    try {
      console.log(`[${this.clientId}] Creating document via Repo directly`);
      console.log(`[${this.clientId}] Content: ${content}`);

      // Create a new document using the repo
      const docId = await this.repo.createDocument(content);

      console.log(`[${this.clientId}] Repo createDocument result:`, docId);
      console.log(
        `[${this.clientId}] Document created successfully with ID: ${docId}`
      );

      return { docId, content };
    } catch (error) {
      console.error(`[${this.clientId}] Failed to create document:`, error);
      throw error;
    }
  }

  async readDocument(docId) {
    if (!this.repo) {
      throw new Error(`[${this.clientId}] No Repo available`);
    }

    try {
      console.log(
        `[${this.clientId}] Reading document from Repo with ID: ${docId}`
      );
      const content = await this.repo.findDocument(docId);

      if (content !== null) {
        console.log(`[${this.clientId}] Found document with ID: ${docId}`);
        console.log(`[${this.clientId}] Content: ${content}`);
        return content;
      } else {
        console.log(`[${this.clientId}] Document not found with ID: ${docId}`);
        return null;
      }
    } catch (error) {
      console.error(`[${this.clientId}] Error reading document:`, error);
      // Return null instead of throwing to match the original VFS behavior
      return null;
    }
  }
}

// Test samod repo API directly (without WebSocket)
async function testSamodRepoAPI() {
  console.log('=== Testing Samod Repo API Directly ===');

  const client1 = new SamodTestClient('SAMOD-1');

  try {
    // Initialize client
    console.log('Initializing client...');
    const init1 = await client1.init();

    if (!init1) {
      throw new Error('Failed to initialize client');
    }

    console.log('Testing direct Samod repo document creation and retrieval...');

    // Client 1 creates a document via Samod repo directly
    const testContent = `Hello from Client 1 via Node.js! Time: ${Date.now()}`;
    const { docId, content: originalContent } = await client1.createDocument(
      null,
      testContent
    );

    console.log(`Document created with ID: ${docId}`);

    // Same client tries to read back the document
    console.log('Same client attempting to read back the document...');
    const retrievedContent = await client1.readDocument(docId);

    if (retrievedContent !== null) {
      console.log(
        '✅ Samod Repo API working! Document was created and retrieved'
      );
      console.log('Original content:', originalContent);
      console.log('Retrieved content:', retrievedContent);

      if (retrievedContent.includes('Hello from Client 1')) {
        console.log('✅ Content matches - repo API working correctly!');
      } else {
        console.log('❌ Content mismatch - repo API issue detected');
      }
    } else {
      console.log('❌ Samod Repo API failed - could not retrieve the document');
    }

    console.log('=== Samod Repo API Test Complete ===\n');
  } catch (error) {
    console.error('❌ Samod repo API test failed:', error);
    throw error;
  }
}

// Test samod document sync directly
async function testSamodDirectSync() {
  console.log('=== Testing Direct Samod Document Sync ===');

  const client1 = new SamodTestClient('SAMOD-1');
  const client2 = new SamodTestClient('SAMOD-2');

  try {
    // Initialize both clients
    console.log('Initializing clients...');
    const init1 = await client1.init();
    const init2 = await client2.init();

    if (!init1 || !init2) {
      throw new Error('Failed to initialize clients');
    }

    // Connect both clients
    console.log('Connecting clients to WebSocket...');
    const conn1 = await client1.connect();
    const conn2 = await client2.connect();

    if (!conn1 || !conn2) {
      throw new Error('Failed to connect clients');
    }

    console.log(
      'Both clients connected, testing direct Samod document creation...'
    );

    // Client 1 creates a document via Samod directly
    const testContent = `Hello from Client 1 via Node.js! Time: ${Date.now()}`;
    const { docId, content: originalContent } = await client1.createDocument(
      null,
      testContent
    );

    console.log(`Document created with ID: ${docId}`);

    // Wait for sync propagation
    console.log('Waiting for sync propagation...');
    await new Promise(resolve => setTimeout(resolve, 3000));

    // Client 2 tries to read the document
    console.log('Client 2 attempting to read the document...');
    const syncedContent = await client2.readDocument(docId);

    if (syncedContent !== null) {
      console.log(
        '✅ Samod Document sync successful! Client 2 found the document'
      );
      console.log('Original content (Client 1):', originalContent);
      console.log('Synced content (Client 2):', syncedContent);

      if (syncedContent.includes('Hello from Client 1')) {
        console.log('✅ Content matches - sync working correctly!');
      } else {
        console.log('❌ Content mismatch - sync issue detected');
      }
    } else {
      console.log(
        '❌ Samod Document sync failed - Client 2 could not find the document'
      );
      console.log(
        'This indicates that samod itself may not be syncing documents properly'
      );
    }

    console.log('=== Samod Direct Sync Test Complete ===\n');
  } catch (error) {
    console.error('❌ Samod sync test failed:', error);
    throw error;
  }
}

export { SamodTestClient, testSamodDirectSync, testSamodRepoAPI };

// Run test if this file is executed directly
if (process.argv[1] === new URL(import.meta.url).pathname) {
  // First test the repo API directly
  testSamodDirectSync()
    .then(() => {
      console.log('Samod repo API test completed');
      process.exit(0);
    })
    .catch(error => {
      console.error('Test failed:', error);
      process.exit(1);
    });
}

