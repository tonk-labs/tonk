/**
 * Environment configuration for Tonk CLI
 * Manages all sensitive configuration values through environment variables
 */

import {config as loadDotenv} from 'dotenv';
import path from 'path';
import {fileURLToPath} from 'url';

// Load .env file if it exists
const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const cliRoot = path.resolve(__dirname, '..');

// Try to load .env from the CLI package root
const envPath = path.join(cliRoot, '.env');
const result = loadDotenv({path: envPath});

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
 * Validates that required environment variables are set
 */
function validateEnvironment(): void {
  const optionalVars = ['TONK_ANALYTICS_API_KEY'];
  const missing = optionalVars.filter(varName => !process.env[varName]);

  if (missing.length > 0) {
    console.warn(
      `Warning: Missing optional environment variables: ${missing.join(', ')}`,
    );
    console.warn(
      'Analytics will be disabled. Set TONK_ANALYTICS_API_KEY to enable usage tracking.',
    );
    console.warn('See SECURITY.md for setup instructions.');
  }
}

/**
 * Load and validate configuration from environment variables
 */
export function loadConfig(): TonkConfig {
  // Validate environment on load
  validateEnvironment();

  return {
    deployment: {
      serviceUrl:
        process.env.TONK_DEPLOYMENT_SERVICE_URL || 'http://localhost:4444',
      defaultRegion: process.env.TONK_DEFAULT_REGION || 'ord',
      defaultMemory: process.env.TONK_DEFAULT_MEMORY || '1gb',
      defaultCpus: process.env.TONK_DEFAULT_CPUS || '1',
    },
    analytics: {
      enabled: process.env.TONK_ANALYTICS_ENABLED !== 'false',
      apiKey: process.env.TONK_ANALYTICS_API_KEY,
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
  } catch (error) {
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
  return {
    enabled: config.analytics.enabled && !!config.analytics.apiKey,
    apiKey: config.analytics.apiKey,
    host: config.analytics.host,
  };
}

/**
 * Get server configuration
 */
export function getServerConfig(): {defaultUrl: string; defaultPort: number} {
  return {
    defaultUrl: config.server.defaultUrl,
    defaultPort: config.server.defaultPort,
  };
}
