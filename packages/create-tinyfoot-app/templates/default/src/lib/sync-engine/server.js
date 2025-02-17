import { Server } from '@hocuspocus/server';
import { Redis } from '@hocuspocus/extension-redis';
import { Database } from '@hocuspocus/extension-database';
import express from 'express';
import expressWebsockets from 'express-ws';
import cors from 'cors';
import { createClient } from 'redis';
import pg from 'pg';
import { config } from './config.js';
import { PODPCDPackage } from '@pcd/pod-pcd';

const { Pool } = pg;

// Initialize Redis client
const redisClient = createClient({
  url: `redis://${config.REDIS.HOST}:${config.REDIS.PORT}`,
});

redisClient.on('error', (err) => {
  console.error('Redis Client Error:', err);
});

redisClient.on('connect', () => {
  console.log('Redis Client Connected');
});

await redisClient.connect();

const ID_SERVICE_KEY_URL = `${config.ID_SERVICE_URL}/.well-known/eddsa-key.json`;

const fetchIdServiceKey = async (retries = 3) => {
  for (let i = 0; i <= retries; i++) {
    try {
      console.log(`Attempting to fetch fetchIdServiceKey (attempt ${i + 1}/${retries + 1})`);
      const response = await fetch(ID_SERVICE_KEY_URL);
      const data = await response.json();
      return data.publicKey;
    } catch (err) {
      console.error(`Failed to fetch ID service key (attempt ${i + 1}/${retries + 1}):`, err);
      // Wait for 3 seconds before retrying
      await new Promise(resolve => setTimeout(resolve, 3000));
      if (i === retries) {
        console.error('Max retries reached. Exiting...');
        process.exit(1);
      }
    }
  }
};

let idServicePublicKey;
try {
  idServicePublicKey = await fetchIdServiceKey();
  console.log('Successfully fetched ID service key:', idServicePublicKey);
} catch (err) {
  console.error('Failed to fetch ID service key after retries:', err);
  process.exit(1);
}

// Initialize Postgres connection pool
const initializePool = async (retries = 5, delay = 5000) => {
  for (let i = 0; i < retries; i++) {
    try {
      const pool = new Pool({
        user: config.POSTGRES.USER,
        host: config.POSTGRES.HOST,
        database: config.POSTGRES.DATABASE,
        password: config.POSTGRES.PASSWORD,
        port: config.POSTGRES.PORT,
      });

      await pool.query('SELECT 1');
      console.log('Successfully connected to PostgreSQL');
      return pool;
    } catch (err) {
      console.error(`Failed to connect to PostgreSQL (attempt ${i + 1}/${retries}):`, err);
      if (i === retries - 1) throw err;
      await new Promise(resolve => setTimeout(resolve, delay));
    }
  }
};

const pool = await initializePool();

// Configure Hocuspocus server
const server = Server.configure({
  extensions: [
    new Redis({
      host: config.REDIS.HOST,
      port: config.REDIS.PORT,
      prefix: 'hocuspocus:',
    }),
    new Database({
      fetch: async ({ documentName }) => {
        try {
          const res = await pool.query(
            'SELECT state FROM yjs_spaces WHERE space_name = $1',
            [documentName]
          );
          if (res.rows.length === 0) {
            return null;
          }
          return new Uint8Array(res.rows[0].state);
        } catch (err) {
          console.error('Error fetching document state:', err);
          throw err;
        }
      },
      store: async ({ documentName, state }) => {
        try {
          const updatedAt = new Date();
          await pool.query(
            `INSERT INTO yjs_spaces (space_name, state, updated_at)
             VALUES ($1, $2, $3)
             ON CONFLICT (space_name)
             DO UPDATE SET state = EXCLUDED.state, updated_at = EXCLUDED.updated_at`,
            [documentName, Buffer.from(state), updatedAt]
          );
          console.log(`Document ${documentName} stored successfully`);
        } catch (err) {
          console.error('Error storing document state:', err);
          throw err;
        }
      },
    }),
  ],

  // Simplified authentication
  async onAuthenticate(data) {
    try {
      // return true;
      const tokenData = JSON.parse(data.token);
      const { userId, userPOD, vibeIds, vibesPOD, documentId, parentVibeId } = tokenData;

      console.log('[server] documentId:', documentId);

      // basic validation
      if (!userId || !userPOD || !documentId) {
        console.log('Missing required authentication fields. Missing:', {
          userId,
          userPOD,
          documentId,
        });

        throw new Error('Missing required authentication fields');
      }

      console.log('Authenticating user access:', {
        userId,
        userPOD,
        documentId,
      });

      // verify POD signatures and claims
      const userClaim = await getClaimFromPOD(userPOD);

      if (idServicePublicKey !== await getSignerPublicKeyFromPOD(userPOD)) {
        console.log('POD signer public key does not match ID service key');
        throw new Error('Invalid signer public key');
      }

      // Arbitrary authentication logic (we can customize checks freely)
      if (userId !== userClaim.userId.value) {
        throw new Error('UserID in POD does not match userID from Engelbart app');
      }
      const oneDayAgo = new Date(Date.now() - 24 * 60 * 60 * 1000);
      if (new Date(userClaim.timestamp.value) < oneDayAgo) {
        throw new Error('POD timestamp is older than 1 day');
      }

      if (documentId.split('/')[0] === 'root') return;

      // verify vibe access
      if (!vibeIds || !vibesPOD) {
        console.log('Missing required authentication fields. Missing:', {
          vibeIds,
          vibesPOD,
        });

        throw new Error('Missing required authentication fields');
      }

      const vibeClaim = await getClaimFromPOD(vibesPOD);
      if (idServicePublicKey !== await getSignerPublicKeyFromPOD(vibesPOD)) {
        console.log('POD signer public key does not match ID service key');
        throw new Error('Invalid signer public key');
      }

      const attributeIds = JSON.parse(vibeClaim.attestations.value);
      if (!attributeIds.some(attr => attr.attribute_id === parentVibeId)) {
        throw new Error('User does not have required vibe access');
      }
    } catch (err) {
      console.error('Authentication error:', err);
      throw err;
    }
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
  console.log(`Sync server running on ws://${config.REDIS.HOST}:${PORT}`);
});

async function getClaimFromPOD(serialisedPCDString) {
  const serialisedPCD = JSON.parse(serialisedPCDString);
  const deserializedPCD = await PODPCDPackage.deserialize(serialisedPCD.pcd);
  const isVerified = await PODPCDPackage.verify(deserializedPCD);
  if (!isVerified) {
    throw new Error('Invalid PCD signature');
  }
  return deserializedPCD.claim.entries;
}

async function getSignerPublicKeyFromPOD(serialisedPCDString) {
  const serialisedPCD = JSON.parse(serialisedPCDString);
  const deserializedPCD = await PODPCDPackage.deserialize(serialisedPCD.pcd);
  return deserializedPCD.claim.signerPublicKey;
}
