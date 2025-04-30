#!/usr/bin/env node

import { configureSyncEngine } from '@tonk/keepsync';
import { BrowserWebSocketClientAdapter } from "@automerge/automerge-repo-network-websocket";
import { NodeFSStorageAdapter } from "@automerge/automerge-repo-storage-nodefs";

// Colors for console output
const colors = {
  success: '\x1b[32m%s\x1b[0m', // Green
  error: '\x1b[31m%s\x1b[0m',   // Red
  info: '\x1b[36m%s\x1b[0m',    // Cyan
  highlight: '\x1b[35m%s\x1b[0m', // Purple
};

// Configuration for connecting to the server
const SERVER_URL = process.env.SERVER_URL || 'localhost:7777';

/**
 * Recursively prints the document tree structure
 * @param {Object} repo - The Automerge repository
 * @param {string} nodeId - The ID of the current node
 * @param {string} indent - Current indentation string for pretty printing
 * @param {string} path - Current path in the tree
 */
async function printDocTree(repo, nodeId, indent = '', path = '/') {
  try {
    const handle = repo.find(nodeId);
    const doc = await handle.doc();
    
    if (!doc) {
      console.log(`${indent}${path} [Not found or empty]`);
      return;
    }
    
    const type = doc.type || 'unknown';
    const timestamp = doc.timestamps?.modified ? new Date(doc.timestamps.modified).toISOString() : 'unknown';
    
    console.log(`${indent}${path} [${type}] - Modified: ${timestamp}`);
    
    if (doc.children && Array.isArray(doc.children)) {
      for (const child of doc.children) {
        const childName = child.name || 'unnamed';
        const childPath = `${path === '/' ? '' : path}/${childName}`;
        
        if (child.pointer && child.type === 'dir') {
          // Recursively print directory contents
          await printDocTree(repo, child.pointer, `${indent}  `, childPath);
        } else if (child.type === 'doc') {
          // Print document info
          const docTimestamp = child.timestamps?.modified ? new Date(child.timestamps.modified).toISOString() : 'unknown';
          console.log(`${indent}  ${childPath} [${child.type}] - Modified: ${docTimestamp}`);
          
          if (child.pointer) {
            try {
              // Optionally fetch and print document content
              const docHandle = repo.find(child.pointer);
              const docContent = await docHandle.doc();
              console.log(`${indent}    Content: ${JSON.stringify(docContent, null, 2).substring(0, 100)}${JSON.stringify(docContent).length > 100 ? '...' : ''}`);
            } catch (err) {
              console.log(`${indent}    Error fetching document content: ${err.message}`);
            }
          }
        }
      }
    }
  } catch (error) {
    console.log(colors.error, `Error traversing node ${nodeId}: ${error.message}`);
  }
}

/**
 * Dumps the entire file tree structure from the repository
 */
async function dumpFileTree() {
  console.log(colors.info, '🚀 Starting file tree dump');
  console.log(`Connecting to server at: ${SERVER_URL}`);

  try {
    // Initialize the sync engine with the server
    const syncEngine = configureSyncEngine({
      url: `http://${SERVER_URL}`,
      storage: new NodeFSStorageAdapter(),
      network: [new BrowserWebSocketClientAdapter(`ws://${SERVER_URL}/sync`)]
    });

    await syncEngine.whenReady();
    console.log(colors.success, '✓ Sync engine initialized');
    
    // Extract repo and rootId from the sync engine
    const repo = syncEngine.getRepo();
    const rootId = syncEngine.getRootId();
    
    console.log(colors.highlight, `Root Document ID: ${rootId}`);
    
    // Dump the entire tree structure
    console.log(colors.info, '\n📁 Document Tree Structure:');
    await printDocTree(repo, rootId);
    
    console.log(colors.success, '\n✅ File tree dump completed');
  } catch (error) {
    console.log(colors.error, '✗ File tree dump failed:');
    console.error(error);
  }
}

// Check if the server is running before starting
async function checkServerAndRun() {
  try {
    const axios = await import('axios');
    await axios.default.get(`http://${SERVER_URL}/ping`);
    console.log(colors.info, '✓ Server is running');
    await dumpFileTree();
    process.exit(0);
  } catch (error) {
    console.log(colors.error, '✗ Server is not running at ' + SERVER_URL);
    console.log('Please start the server first with: node start-server.js');
    process.exit(1);
  }
}

checkServerAndRun(); 