// import {describe, it, expect, beforeEach, afterEach} from 'vitest';
// import {create} from 'zustand';
// import {SyncEngine} from '../../src/engine';
// import {sync} from '../../src/middleware/sync';
// import {TestWebSocketServer} from '../engine/test-helpers';
// import 'fake-indexeddb/auto';

// /**
//  * Integration test for Zustand synchronization using the sync middleware
//  *
//  * This test verifies that two Zustand stores using the sync middleware
//  * can successfully synchronize state changes through the singleton SyncEngine.
//  */
// describe('Zustand Sync Middleware Integration', () => {
//   // Configuration constants
//   const WEBSOCKET_PORT = 3032;
//   const TEST_DB_1 = 'test_integration_db_1';
//   const TEST_DB_2 = 'test_integration_db_2';
//   const DOC_ID = 'counter-doc';
//   const CONNECTION_TIMEOUT = 100;
//   const TEARDOWN_TIMEOUT = 300;
//   const SYNC_TIMEOUT = 2000;
//   const DEBUG = process.env.DEBUG === 'true';

//   // Test fixture variables
//   let wss: TestWebSocketServer;
//   let syncCount1 = 0;
//   let syncCount2 = 0;

//   // Debug utility function
//   const debug = (...args: any[]) => {
//     if (DEBUG) console.log(...args);
//   };

//   // Interface for our test store
//   interface CounterStore {
//     count: number;
//     increment: () => void;
//     decrement: () => void;
//   }

//   // Helper function to set up the test environment
//   async function setupTestEnvironment() {
//     // Create WebSocket server for communication
//     const wss = new TestWebSocketServer(WEBSOCKET_PORT);

//     // Reset counts
//     syncCount1 = 0;
//     syncCount2 = 0;

//     // Reset the singleton instance to ensure clean test state
//     SyncEngine.resetInstance();

//     return {
//       wss,
//       async teardown() {
//         // Reset the singleton instance
//         SyncEngine.resetInstance();

//         // Close the WebSocket server
//         await wss.close();

//         // Wait to ensure connections are properly closed
//         await new Promise(resolve => setTimeout(resolve, TEARDOWN_TIMEOUT));

//         // Delete test databases
//         indexedDB.deleteDatabase(TEST_DB_1);
//         indexedDB.deleteDatabase(TEST_DB_2);
//       },
//     };
//   }

//   beforeEach(async () => {
//     const env = await setupTestEnvironment();
//     wss = env.wss;
//   });

//   afterEach(async () => {
//     // Make sure the singleton is reset
//     SyncEngine.resetInstance();

//     // Close WebSocket server if it exists
//     if (wss) await wss.close();

//     // Wait to ensure all connections are properly closed
//     await new Promise(resolve => setTimeout(resolve, TEARDOWN_TIMEOUT));

//     // Clean up databases
//     indexedDB.deleteDatabase(TEST_DB_1);
//     indexedDB.deleteDatabase(TEST_DB_2);
//   });

//   it('syncs state between two stores using the singleton sync engine', async () => {
//     // Configure first sync engine instance
//     await SyncEngine.configureInstance({
//       url: `ws://localhost:${WEBSOCKET_PORT}`,
//       dbName: TEST_DB_1,
//       name: 'engine1',
//       onSync: docId => {
//         debug(`Engine 1 sync triggered for ${docId}, count: ${syncCount1++}`);
//       },
//     });

//     // Create first store with sync middleware
//     const useStore1 = create<CounterStore>()(
//       sync(
//         set => ({
//           count: 0,
//           increment: () => set(state => ({count: state.count + 1})),
//           decrement: () => set(state => ({count: state.count - 1})),
//         }),
//         {docId: DOC_ID},
//       ),
//     );

//     // Wait for store 1 to initialize
//     await new Promise(resolve => setTimeout(resolve, CONNECTION_TIMEOUT));

//     // Reset the singleton for second store
//     SyncEngine.resetInstance();

//     // Configure second sync engine instance
//     await SyncEngine.configureInstance({
//       url: `ws://localhost:${WEBSOCKET_PORT}`,
//       dbName: TEST_DB_2,
//       name: 'engine2',
//       onSync: docId => {
//         debug(`Engine 2 sync triggered for ${docId}, count: ${syncCount2++}`);
//       },
//     });

//     // Create second store with sync middleware
//     const useStore2 = create<CounterStore>()(
//       sync(
//         set => ({
//           count: 0,
//           increment: () => set(state => ({count: state.count + 1})),
//           decrement: () => set(state => ({count: state.count - 1})),
//         }),
//         {docId: DOC_ID},
//       ),
//     );

//     // Wait for store 2 to initialize and connect
//     await new Promise(resolve => setTimeout(resolve, CONNECTION_TIMEOUT));

//     // Initial states should eventually sync to match
//     await waitForSync(
//       () => useStore1.getState().count === useStore2.getState().count,
//       SYNC_TIMEOUT,
//     );

//     // Update store 1
//     debug('Incrementing store 1');
//     useStore1.getState().increment();
//     expect(useStore1.getState().count).toBe(1);

//     // Wait for sync to happen
//     await waitForSync(() => useStore2.getState().count === 1, SYNC_TIMEOUT);

//     // Store 2 should now have the updated value
//     expect(useStore2.getState().count).toBe(1);

//     // Update store 2
//     useStore2.getState().increment();
//     expect(useStore2.getState().count).toBe(2);

//     // Wait for sync back to store 1
//     await waitForSync(() => useStore1.getState().count === 2, SYNC_TIMEOUT);

//     // Store 1 should have the updated value
//     expect(useStore1.getState().count).toBe(2);

//     // Check that decrement works too
//     useStore2.getState().decrement();
//     expect(useStore2.getState().count).toBe(1);

//     // Wait for final sync
//     await waitForSync(() => useStore1.getState().count === 1, SYNC_TIMEOUT);
//     expect(useStore1.getState().count).toBe(1);

//     // Verify sync counts
//     expect(syncCount1).toBeGreaterThan(0);
//     expect(syncCount2).toBeGreaterThan(0);
//   });

//   it('handles concurrent updates correctly with sync middleware', async () => {
//     // Configure first sync engine instance
//     await SyncEngine.configureInstance({
//       url: `ws://localhost:${WEBSOCKET_PORT}`,
//       dbName: TEST_DB_1,
//       name: 'engine1-concurrent',
//     });

//     // Create first store
//     const useStore1 = create<CounterStore>()(
//       sync(
//         set => ({
//           count: 0,
//           increment: () => set(state => ({count: state.count + 1})),
//           decrement: () => set(state => ({count: state.count - 1})),
//         }),
//         {docId: 'counter-doc-concurrent'},
//       ),
//     );

//     // Wait for store 1 to initialize
//     await new Promise(resolve => setTimeout(resolve, CONNECTION_TIMEOUT));

//     // Reset for second store
//     SyncEngine.resetInstance();

//     // Configure second sync engine instance
//     await SyncEngine.configureInstance({
//       url: `ws://localhost:${WEBSOCKET_PORT}`,
//       dbName: TEST_DB_2,
//       name: 'engine2-concurrent',
//     });

//     // Create second store
//     const useStore2 = create<CounterStore>()(
//       sync(
//         set => ({
//           count: 0,
//           increment: () => set(state => ({count: state.count + 1})),
//           decrement: () => set(state => ({count: state.count - 1})),
//         }),
//         {docId: 'counter-doc-concurrent'},
//       ),
//     );

//     // Wait for store 2 to initialize
//     await new Promise(resolve => setTimeout(resolve, CONNECTION_TIMEOUT));

//     // Make concurrent updates from both stores
//     useStore1.getState().increment();
//     useStore2.getState().increment();

//     // Wait for sync to stabilize
//     await waitForSync(
//       () =>
//         useStore1.getState().count === 1 && useStore2.getState().count === 1,
//       SYNC_TIMEOUT,
//     );

//     // Both stores should have count = 1
//     expect(useStore1.getState().count).toBe(1);
//     expect(useStore2.getState().count).toBe(1);
//   });

//   it('syncs data within acceptable time limits using sync middleware', async () => {
//     // Configure first sync engine instance
//     await SyncEngine.configureInstance({
//       url: `ws://localhost:${WEBSOCKET_PORT}`,
//       dbName: TEST_DB_1,
//       name: 'engine1-perf',
//     });

//     // Create first store
//     const useStore1 = create<CounterStore>()(
//       sync(
//         set => ({
//           count: 0,
//           increment: () => set(state => ({count: state.count + 1})),
//           decrement: () => set(state => ({count: state.count - 1})),
//         }),
//         {docId: 'counter-doc-perf'},
//       ),
//     );

//     // Wait for store 1 to initialize
//     await new Promise(resolve => setTimeout(resolve, CONNECTION_TIMEOUT));

//     // Reset for second store
//     SyncEngine.resetInstance();

//     // Configure second sync engine instance
//     await SyncEngine.configureInstance({
//       url: `ws://localhost:${WEBSOCKET_PORT}`,
//       dbName: TEST_DB_2,
//       name: 'engine2-perf',
//     });

//     // Create second store
//     const useStore2 = create<CounterStore>()(
//       sync(
//         set => ({
//           count: 0,
//           increment: () => set(state => ({count: state.count + 1})),
//           decrement: () => set(state => ({count: state.count - 1})),
//         }),
//         {docId: 'counter-doc-perf'},
//       ),
//     );

//     // Wait for store 2 to initialize
//     await new Promise(resolve => setTimeout(resolve, CONNECTION_TIMEOUT));

//     const startTime = Date.now();
//     useStore1.getState().increment();

//     // Wait for sync
//     await waitForSync(() => useStore2.getState().count === 1, SYNC_TIMEOUT);

//     const syncTime = Date.now() - startTime;
//     debug(`Sync completed in ${syncTime}ms`);

//     // Assert that sync happens within acceptable time (adjust threshold as needed)
//     expect(syncTime).toBeLessThan(500);
//   });
// });

// /**
//  * Helper function to wait for a condition with timeout
//  *
//  * @param condition Function that returns true when the condition is met
//  * @param timeout Maximum time to wait in milliseconds
//  * @param checkInterval Interval between condition checks in milliseconds
//  * @returns Promise that resolves when condition is met or rejects on timeout
//  */
// async function waitForSync(
//   condition: () => boolean,
//   timeout: number = 1000,
//   checkInterval: number = 100,
// ): Promise<void> {
//   const startTime = Date.now();
//   let checkCount = 0;

//   while (Date.now() - startTime < timeout) {
//     checkCount++;
//     if (condition()) {
//       return;
//     }
//     await new Promise(resolve => setTimeout(resolve, checkInterval));
//   }

//   throw new Error(`Sync timeout after ${timeout}ms - condition never met`);
// }
