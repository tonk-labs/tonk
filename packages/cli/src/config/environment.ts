/**
 * Environment configuration for Tonk CLI
 * Manages all sensitive configuration values through environment variables
 */

import { config as loadDotenv } from 'dotenv';
import { existsSync } from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

// Load .env file if it exists
const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const cliRoot = path.resolve(__dirname, '..');

// Try to load .env from the CLI package root only if it exists
const envPath = path.join(cliRoot, '.env');
if (existsSync(envPath)) {
  loadDotenv({ path: envPath });
}

export interface TonkConfig {
  deployment: {
    serviceUrl: string;
    defaultRegion: string;
    defaultMemory: string;
    defaultCpus: string;
  };
  analytics: {
    enabled: boolean;
    apiKey?: string;
    host: string;
  };
  server: {
    defaultUrl: string;
    defaultPort: number;
  };
  logging: {
    level: 'debug' | 'info' | 'warn' | 'error';
  };
}

/**
 * Load and validate configuration from environment variables
 */
export function loadConfig(): TonkConfig {
  return {
    deployment: {
      serviceUrl:
        process.env.TONK_DEPLOYMENT_SERVICE_URL ||
        'http://ec2-51-20-65-254.eu-north-1.compute.amazonaws.com:4444',
      defaultRegion: process.env.TONK_DEFAULT_REGION || 'ord',
      defaultMemory: process.env.TONK_DEFAULT_MEMORY || '1gb',
      defaultCpus: process.env.TONK_DEFAULT_CPUS || '1',
    },
    analytics: {
      enabled: process.env.TONK_ANALYTICS_ENABLED !== 'false',
      apiKey:
        process.env.TONK_ANALYTICS_API_KEY ||
        'phc_dPEh0Tb5GFMZtykYV6Yg8VEHqJeAutrL7frEMYKmRuW',
      host: process.env.TONK_ANALYTICS_HOST || 'https://eu.i.posthog.com',
    },
    server: {
      defaultUrl: process.env.TONK_SERVER_URL || 'http://localhost:7777',
      defaultPort: parseInt(process.env.TONK_SERVER_PORT || '7777', 10),
    },
    logging: {
      level: (process.env.TONK_LOG_LEVEL as any) || 'info',
    },
  };
}

/**
 * Global configuration instance
 */
export const config = loadConfig();

/**
 * Get deployment service URL with validation
 */
export function getDeploymentServiceUrl(): string {
  const url = config.deployment.serviceUrl;

  // Validate URL format
  try {
    new URL(url);
  } catch {
    throw new Error(`Invalid deployment service URL: ${url}`);
  }

  return url;
}

/**
 * Get analytics configuration
 */
export function getAnalyticsConfig(): {
  enabled: boolean;
  apiKey?: string;
  host: string;
} {
  const apiKey = config.analytics.apiKey;

  return {
    enabled: config.analytics.enabled && !!config.analytics.apiKey,
    ...(apiKey && { apiKey }),
    host: config.analytics.host,
  };
}

/**
 * Get server configuration
 */
export function getServerConfig(): { defaultUrl: string; defaultPort: number } {
  return {
    defaultUrl: config.server.defaultUrl,
    defaultPort: config.server.defaultPort,
  };
}
