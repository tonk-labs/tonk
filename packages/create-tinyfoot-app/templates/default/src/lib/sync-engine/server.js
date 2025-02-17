import { Server } from '@hocuspocus/server';
import { Redis } from '@hocuspocus/extension-redis';
import { Database } from '@hocuspocus/extension-database';
import express from 'express';
import expressWebsockets from 'express-ws';
import cors from 'cors';
import { createClient } from 'redis';
import sqlite3 from 'sqlite3';
import { open } from 'sqlite';
import { config } from '../../../config.js';

// Initialize Redis client
const redisClient = createClient({
  url: `redis://${config.SYNC.REDIS.HOST}:${config.SYNC.REDIS.PORT}`,
});

redisClient.on('error', (err) => {
  console.error('Redis Client Error:', err);
});

redisClient.on('connect', () => {
  console.log('Redis Client Connected');
});

await redisClient.connect();

// Initialize SQLite database
const initializeDatabase = async () => {
  try {
    const db = await open({
      filename: './data/sync.db',
      driver: sqlite3.Database
    });

    // Create the yjs_spaces table if it doesn't exist
    await db.exec(`
      CREATE TABLE IF NOT EXISTS yjs_spaces (
        space_name TEXT PRIMARY KEY,
        state BLOB,
        updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
      )
    `);

    console.log('Successfully connected to SQLite database');
    return db;
  } catch (err) {
    console.error('Failed to initialize SQLite database:', err);
    throw err;
  }
};

const db = await initializeDatabase();

// Configure Hocuspocus server
const server = Server.configure({
  extensions: [
    new Redis({
      host: config.SYNC.REDIS.HOST,
      port: config.SYNC.REDIS.PORT,
      prefix: 'hocuspocus:',
    }),
    new Database({
      fetch: async ({ documentName }) => {
        try {
          const row = await db.get(
            'SELECT state FROM yjs_spaces WHERE space_name = ?',
            documentName
          );
          if (!row) {
            return null;
          }
          return new Uint8Array(row.state);
        } catch (err) {
          console.error('Error fetching document state:', err);
          throw err;
        }
      },
      store: async ({ documentName, state }) => {
        try {
          await db.run(
            `INSERT INTO yjs_spaces (space_name, state, updated_at)
             VALUES (?, ?, CURRENT_TIMESTAMP)
             ON CONFLICT(space_name)
             DO UPDATE SET 
               state = excluded.state,
               updated_at = CURRENT_TIMESTAMP`,
            documentName,
            Buffer.from(state)
          );
          console.log(`Document ${documentName} stored successfully`);
        } catch (err) {
          console.error('Error storing document state:', err);
          throw err;
        }
      },
    }),
  ],

  // Authentication hook for future implementation
  async onAuthenticate(data) {
    // No authentication required for now
    return true;
  },

  async onLoadDocument(data) {
    console.log(`Loading document: ${data.documentName}`);
  },

  async onStoreDocument(data) {
    console.log(`Storing document: ${data.documentName}`);
  },
});

// Set up Express app with WebSocket support
const { app } = expressWebsockets(express());
app.use(cors());

// WebSocket endpoint for Hocuspocus
app.ws('/', (websocket, request) => {
  server.handleConnection(websocket, request);
});

const PORT = 3002;
app.listen(PORT, () => {
  console.log(`Sync server running on ws://${config.SYNC.REDIS.HOST}:${PORT}`);
});
