import dotenv from 'dotenv';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const NODE_ENV = process.env.NODE_ENV;
const envFileName = `.env.${NODE_ENV}`;

dotenv.config({ path: path.resolve(__dirname, envFileName) });

export const config = {
  SYNC: {
    NODE_ENV,
    ID_SERVICE_URL: process.env.ID_SERVICE_URL,
    POSTGRES: {
      USER: process.env.PG_USER,
      HOST: process.env.PG_HOST,
      DATABASE: process.env.PG_DATABASE,
      PASSWORD: process.env.PG_PASSWORD,
      PORT: parseInt(process.env.PG_PORT, 10),
    },
    REDIS: {
      HOST: process.env.REDIS_HOST,
      PORT: parseInt(process.env.REDIS_PORT, 10),
    },
    PASSKEY: {
      RP_ID: process.env.RP_ID,
      RP_ORIGIN: process.env.RP_ORIGIN,
    },
  },
};
