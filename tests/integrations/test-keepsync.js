#!/usr/bin/env node

import { configureSyncEngine, readDoc, writeDoc, sync } from '@tonk/keepsync';
// import { readDoc, writeDoc } from '@tonk/keepsync/dist/middleware/sync.js';
import { BrowserWebSocketClientAdapter } from "@automerge/automerge-repo-network-websocket";
import { NodeFSStorageAdapter } from "@automerge/automerge-repo-storage-nodefs";
import { exec } from 'child_process';
import util from 'util';

const execPromise = util.promisify(exec);

// Configuration for connecting to the test server
const SERVER_URL = process.env.SERVER_URL || 'localhost:7777';
const TEST_DOCUMENT_PATH = '/integration-test/document';
const TEST_DATA = { message: 'Hello from integration test', timestamp: Date.now() };

// Colors for console output
const colors = {
  success: '\x1b[32m%s\x1b[0m', // Green
  error: '\x1b[31m%s\x1b[0m',   // Red
  info: '\x1b[36m%s\x1b[0m',    // Cyan
};

import { createStore } from "zustand/vanilla";

// Create a synced store for todos


/**
 * Runs the integration test for keepsync with the Tonk server
 */
async function runTest() {
  console.log(colors.info, '🚀 Starting keepsync integration test');
  console.log(`Connecting to server at: ${SERVER_URL}`);

  try {
    // Initialize the sync engine with the test server
    const syncEngine = configureSyncEngine({
      url: `http://${SERVER_URL}`,
      storage: new NodeFSStorageAdapter(),
      network: [new BrowserWebSocketClientAdapter(`ws://${SERVER_URL}/sync`)]
    });

    await syncEngine.whenReady();

    console.log(colors.info, '✓ Sync engine initialized');
    
    // Write a test document
    console.log('Writing test document...');
    await writeDoc(TEST_DOCUMENT_PATH, TEST_DATA);
    console.log(colors.success, '✓ Document written successfully');

    // Read the document back to verify
    console.log('Reading test document...');
    const retrievedDoc = await readDoc(TEST_DOCUMENT_PATH);
    
    if (!retrievedDoc) {
      throw new Error('Failed to read the document back');
    }
    
    console.log('Retrieved document:', retrievedDoc);
    
    // Verify the document content matches what we wrote
    if (retrievedDoc.message === TEST_DATA.message) {
      console.log(colors.success, '✓ Document verified - content matches!');
    } else {
      console.log(colors.error, '✗ Document content mismatch!');
      console.log('Expected:', TEST_DATA);
      console.log('Received:', retrievedDoc);
    }

    // Test successful!
    console.log(colors.success, '🎉 Keepsync integration test completed successfully!');
  } catch (error) {
    console.log(colors.error, '✗ Integration test failed:');
    console.error(error);
  }
}

async function runZustandTest() {
  //first initialize it to zero
  await writeDoc('todo-list', {todos: []});
  const todoStore = createStore(
    sync(
      (set) => ({
        todos: [],

        // Add a new todo
        addTodo: (text) => {
          set((state) => ({
            todos: [
              ...state.todos,
              {
                id: crypto.randomUUID(),
                text,
                completed: false,
              },
            ],
          }));
        },

        // Toggle a todo's completed status
        toggleTodo: (id) => {
          set((state) => ({
            todos: state.todos.map((todo) =>
              todo.id === id ? { ...todo, completed: !todo.completed } : todo
            ),
          }));
        },

        // Delete a todo
        deleteTodo: (id) => {
          set((state) => ({
            todos: state.todos.filter((todo) => todo.id !== id),
          }));
        },
      }),
      {
        // Unique document ID for this store
        docId: "todo-list",
      }
    )
  );
  const state = todoStore.getState();
  let counter = 0;
  return new Promise((resolve, reject) => {
    setInterval(async () => {
      // This is just as an example of how to use the zustand stores in Node
      counter++;
      state.addTodo(`Todo number ${counter}`);
      if (todoStore.getState().todos.length > 1) {
        state.deleteTodo(todoStore.getState().todos[0].id);
      }
      if (counter > 5) {
        clearInterval();
        const result = await readDoc('todo-list');
        console.log(result);
        resolve();
      }
    }, 2000);
  })
}

// Check if the server is running before starting the test
async function checkServerAndRun() {
  try {
    // Use axios instead of node-fetch to check if server is responding
    const axios = await import('axios');
    await axios.default.get(`http://${SERVER_URL}/ping`);
    console.log(colors.info, '✓ Server is running');
    await runTest();
    await runZustandTest();
    process.exit(0);
  } catch (error) {
    console.log(colors.error, '✗ Server is not running at ' + SERVER_URL);
    console.log('Please start the server first with: node start-server.js');
    process.exit(1);
  }
}

checkServerAndRun(); 