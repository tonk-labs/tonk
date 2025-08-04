// import {describe, it, expect, beforeEach, afterEach} from 'vitest';
// import {SyncEngine} from '../../src/engine';
// import {TestWebSocketServer} from './test-helpers';
// import 'fake-indexeddb/auto';
// import * as Automerge from '@automerge/automerge';
// import {DocumentId} from '../../src/engine/types';

// interface SyncedDoc {
//   text?: string;
//   counter?: number;
// }

// describe('SyncEngine', () => {
//   let engine: SyncEngine;
//   let wss: TestWebSocketServer;
//   const TEST_PORT = 3031;
//   const TEST_DB = 'test_db';

//   beforeEach(async () => {
//     // Start WebSocket server first
//     wss = new TestWebSocketServer(TEST_PORT);

//     // Reset the singleton instance before configuring a new one
//     SyncEngine.resetInstance();

//     // Initialize a new engine before each test using the singleton pattern
//     engine = await SyncEngine.configureInstance({
//       url: `ws://localhost:${TEST_PORT}/sync`,
//       dbName: TEST_DB,
//       name: 'engine',
//       onSync: docId => console.log(`Document ${docId} synced`),
//       onError: error => console.error('Sync error:', error),
//     });

//     // Wait for WebSocket connection to be established
//     await new Promise(resolve => setTimeout(resolve, 100));
//   });

//   afterEach(async () => {
//     // Wait for any pending operations to complete
//     await new Promise(resolve => setTimeout(resolve, 100));

//     // Clean up after each test
//     engine.close(); // This also resets the instance internally

//     await wss.close();

//     // Wait for WebSocket to properly close
//     await new Promise(resolve => setTimeout(resolve, 100));

//     // Clear all IndexedDB data
//     indexedDB.deleteDatabase(TEST_DB);
//   });

//   describe('Document Operations', () => {
//     it('should create a new document', async () => {
//       const docId = 'test-doc-1' as DocumentId;
//       const initialContent = {text: 'Hello, World!'};

//       await engine.createDocument(docId, initialContent);
//       const doc = await engine.getDocument(docId);

//       expect(doc).not.toBeNull();
//       expect(doc.text).toBe('Hello, World!');
//     });

//     it('should update an existing document', async () => {
//       const docId = 'test-doc-2' as DocumentId;
//       await engine.createDocument(docId, {text: 'Initial'});

//       await engine.updateDocument(docId, (doc: SyncedDoc) => {
//         doc.text = 'Updated';
//       });

//       const doc = await engine.getDocument(docId);
//       expect(doc.text).toBe('Updated');
//     });

//     it('should handle concurrent updates', async () => {
//       const docId = 'test-doc-3' as DocumentId;
//       await engine.createDocument(docId, {counter: 0});

//       // Simulate concurrent updates
//       await Promise.all([
//         engine.updateDocument(docId, (doc: SyncedDoc) => {
//           if (doc.counter !== undefined) {
//             doc.counter += 1;
//           }
//         }),
//         engine.updateDocument(docId, (doc: SyncedDoc) => {
//           if (doc.counter !== undefined) {
//             doc.counter += 2;
//           }
//         }),
//       ]);

//       const doc = await engine.getDocument(docId);
//       expect(doc.counter).toBe(3);
//     });
//   });

//   describe('Error Handling', () => {
//     it('should throw error when updating non-existent document', async () => {
//       await expect(async () => {
//         await engine.updateDocument(
//           'non-existent' as DocumentId,
//           (doc: SyncedDoc) => {
//             doc.text = 'Should fail';
//           },
//         );
//       }).rejects.toThrow('Document not found');
//     });

//     it('should handle WebSocket connection errors gracefully', async () => {
//       // Clean up existing instance first
//       engine.close();

//       // Create a flag to track if the error callback was called
//       let errorCalled = false;

//       // Configure with invalid WebSocket URL
//       const invalidEngine = await SyncEngine.configureInstance({
//         url: 'ws://invalid-host:9999/sync', // Invalid URL
//         dbName: TEST_DB + '_invalid',
//         name: 'invalid-engine',
//         onError: error => {
//           errorCalled = true;
//           console.log('Expected error received:', error);
//         },
//       });

//       // The engine should still be configured despite the invalid WebSocket URL
//       expect(invalidEngine).toBeDefined();

//       // Wait for the WebSocket connection attempt to fail and trigger the error callback
//       await new Promise(resolve => setTimeout(resolve, 500));

//       // Verify that the error callback was called
//       expect(errorCalled).toBe(true);

//       // Clean up the invalid engine
//       invalidEngine.close();
//       indexedDB.deleteDatabase(TEST_DB + '_invalid');

//       // Reset for other tests
//       SyncEngine.resetInstance();

//       // Reconfigure the main engine for other tests
//       engine = await SyncEngine.configureInstance({
//         url: `ws://localhost:${TEST_PORT}/sync`,
//         dbName: TEST_DB,
//         name: 'engine',
//         onSync: docId => console.log(`Document ${docId} synced`),
//         onError: error => console.error('Sync error:', error),
//       });
//     });
//   });

//   describe('Sync Functionality', () => {
//     it('should sync changes between two engines', async () => {
//       // Clean up the first engine
//       engine.close();

//       // Configure first engine
//       engine = await SyncEngine.configureInstance({
//         url: `ws://localhost:${TEST_PORT}/sync`,
//         dbName: TEST_DB,
//         name: 'engine',
//         onSync: docId => console.log(`Document ${docId} synced`),
//         onError: error => console.error('Sync error:', error),
//       });

//       // Configure second engine in a separate variable
//       let engine2Instance;

//       try {
//         // Create and configure a second engine (handled separately from singleton)
//         SyncEngine.resetInstance(); // Reset the singleton before creating second engine
//         engine2Instance = await SyncEngine.configureInstance({
//           url: `ws://localhost:${TEST_PORT}/sync`,
//           dbName: 'test_db_2',
//           name: 'engine2',
//           onSync: docId => console.log(`Engine 2: Document ${docId} synced`),
//           onError: error => console.error('Engine 2 error:', error),
//         });

//         const docId = 'shared-doc' as DocumentId;
//         await engine.createDocument(docId, {text: 'Original'});

//         // Verify initial document in engine1
//         const doc1 = await engine.getDocument(docId);
//         console.log('Initial doc1:', doc1);
//         expect(doc1.text).toBe('Original');

//         //We need these to be wrapped in setTimeout, await doesn't work given how event loop handing in JS works
//         setTimeout(async () => {
//           // Wait for sync with more retries and longer delay
//           let initialDoc2: Automerge.Doc<SyncedDoc> | null = null;
//           for (let i = 0; i < 15; i++) {
//             await new Promise(resolve => setTimeout(resolve, 200));
//             initialDoc2 = await engine2Instance.getDocument(docId);
//             if (initialDoc2?.text === 'Original') break;
//           }

//           // Ensure the initial document synced before continuing
//           expect(initialDoc2?.text).toBe('Original');
//           console.log('Initial sync verified:', initialDoc2);
//         }, 100);

//         // Only proceed with update after initial sync is verified
//         await engine.updateDocument(docId, (doc: SyncedDoc) => {
//           doc.text = 'Updated from engine 1';
//         });

//         //We need these to be wrapped in setTimeout, await doesn't work given how event loop handing in JS works
//         setTimeout(async () => {
//           // Wait for sync with more retries and longer delay
//           let finalDoc2: Automerge.Doc<SyncedDoc> | null = null;
//           for (let i = 0; i < 15; i++) {
//             await new Promise(resolve => setTimeout(resolve, 200));
//             finalDoc2 = await engine2Instance.getDocument(docId);
//             if (finalDoc2?.text === 'Updated from engine 1') break;
//           }

//           // Verify final state
//           const finalDoc1 = await engine.getDocument(docId);
//           console.log('Final states:', {doc1: finalDoc1, doc2: finalDoc2});

//           expect(finalDoc2?.text).toBe('Updated from engine 1');
//         }, 100);
//       } finally {
//         // Clean up engine2
//         if (engine2Instance) {
//           engine2Instance.close();
//         }
//         await new Promise(resolve => setTimeout(resolve, 100));
//         indexedDB.deleteDatabase('test_db_2');

//         // Reset singleton for cleanup
//         SyncEngine.resetInstance();

//         // Reconfigure main engine for other tests
//         engine = await SyncEngine.configureInstance({
//           url: `ws://localhost:${TEST_PORT}/sync`,
//           dbName: TEST_DB,
//           name: 'engine',
//           onSync: docId => console.log(`Document ${docId} synced`),
//           onError: error => console.error('Sync error:', error),
//         });
//       }
//     });
//   });
// });
