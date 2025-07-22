import { PostHog } from 'posthog-node';
import os from 'os';
import fs from 'fs';
import path from 'path';
import { CLI_VERSION } from '../utils/version.js';
import { getAnalyticsConfig } from '../config/environment.js';

// Fallback machine ID generation if node-machine-id is not available
function generateFallbackMachineId(): string {
  const hostname = os.hostname();
  const platform = os.platform();
  const arch = os.arch();
  const userInfo = os.userInfo();

  // Create a simple hash from system info
  const info = `${hostname}-${platform}-${arch}-${userInfo.username}`;
  let hash = 0;
  for (let i = 0; i < info.length; i++) {
    const char = info.charCodeAt(i);
    hash = (hash << 5) - hash + char;
    hash = hash & hash; // Convert to 32-bit integer
  }
  return Math.abs(hash).toString(16);
}

/**
 * Get CLI version from shared version constant
 */
function getCliVersion(): string {
  return CLI_VERSION || 'unknown';
}

/**
 * Detect workspace type (pnpm, npm, yarn)
 */
function detectWorkspaceType(): string {
  try {
    const cwd = process.cwd();
    if (fs.existsSync(path.join(cwd, 'pnpm-lock.yaml'))) return 'pnpm';
    if (fs.existsSync(path.join(cwd, 'yarn.lock'))) return 'yarn';
    if (fs.existsSync(path.join(cwd, 'package-lock.json'))) return 'npm';
    return 'unknown';
  } catch (error) {
    return 'unknown';
  }
}

/**
 * Check if tonk.config.json exists in current directory
 */
function hasTonkConfig(): boolean {
  try {
    return fs.existsSync(path.join(process.cwd(), 'tonk.config.json'));
  } catch (error) {
    return false;
  }
}

/**
 * Get enhanced system context for analytics
 */
function getSystemContext(): Record<string, any> {
  return {
    platform: os.platform(),
    arch: os.arch(),
    nodeVersion: process.version,
    cliVersion: getCliVersion(),
    workspaceType: detectWorkspaceType(),
    hasTonkConfig: hasTonkConfig(),
    timestamp: new Date().toISOString(),
  };
}

// Singleton PostHog instance
let posthog: PostHog | null = null;

/**
 * Initialize PostHog analytics
 */
export function initAnalytics(): PostHog | null {
  if (!posthog) {
    const analyticsConfig = getAnalyticsConfig();

    // Only initialize if analytics is enabled and API key is provided
    if (!analyticsConfig.enabled || !analyticsConfig.apiKey) {
      console.debug('Analytics disabled or no API key provided');
      return null;
    }

    posthog = new PostHog(analyticsConfig.apiKey, {
      host: analyticsConfig.host,
    });
  }
  return posthog;
}

/**
 * Get a unique machine identifier for analytics
 */
function getMachineId(): string {
  try {
    // Try to use node-machine-id if available
    const { machineIdSync } = require('node-machine-id');
    return machineIdSync();
  } catch (error) {
    // Fallback to system-based ID
    return generateFallbackMachineId();
  }
}

/**
 * Track a command execution
 */
export function trackCommand(
  commandName: string,
  options: Record<string, any> = {},
  additionalProps: Record<string, any> = {}
): void {
  try {
    const analytics = initAnalytics();
    if (!analytics) {
      return; // Analytics disabled
    }

    const machineId = getMachineId();

    analytics.capture({
      distinctId: machineId,
      event: 'command_executed',
      properties: {
        command: commandName,
        options,
        ...getSystemContext(),
        ...additionalProps,
      },
    });
  } catch (error) {
    // Silently fail analytics to not interrupt CLI functionality
    console.debug('Analytics tracking failed:', error);
  }
}

/**
 * Track command success
 */
export function trackCommandSuccess(
  commandName: string,
  duration?: number,
  additionalProps: Record<string, any> = {}
): void {
  try {
    const analytics = initAnalytics();
    if (!analytics) {
      return; // Analytics disabled
    }

    const machineId = getMachineId();

    analytics.capture({
      distinctId: machineId,
      event: 'command_success',
      properties: {
        command: commandName,
        duration_ms: duration,
        ...getSystemContext(),
        ...additionalProps,
      },
    });
  } catch (error) {
    console.debug('Analytics tracking failed:', error);
  }
}

/**
 * Track command error
 */
export function trackCommandError(
  commandName: string,
  error: Error | string,
  duration?: number,
  additionalProps: Record<string, any> = {}
): void {
  try {
    const analytics = initAnalytics();
    if (!analytics) {
      return; // Analytics disabled
    }

    const machineId = getMachineId();

    analytics.capture({
      distinctId: machineId,
      event: 'command_error',
      properties: {
        command: commandName,
        error: error instanceof Error ? error.message : error,
        error_type: error instanceof Error ? error.constructor.name : 'string',
        error_stack: error instanceof Error ? error.stack : undefined,
        duration_ms: duration,
        ...getSystemContext(),
        ...additionalProps,
      },
    });
  } catch (analyticsError) {
    console.debug('Analytics tracking failed:', analyticsError);
  }
}

/**
 * Shutdown analytics and flush any pending events
 */
export async function shutdownAnalytics(): Promise<void> {
  if (posthog) {
    try {
      await posthog.shutdown();
    } catch (error) {
      console.debug('Analytics shutdown failed:', error);
    }
  }
}

/**
 * Wrapper function to track command execution with timing
 */
export function withAnalytics<T extends any[], R>(
  commandName: string,
  fn: (...args: T) => R | Promise<R>,
  optionsExtractor?: (...args: T) => Record<string, any>
): (...args: T) => Promise<R> {
  return async (...args: T): Promise<R> => {
    const startTime = Date.now();
    const options = optionsExtractor ? optionsExtractor(...args) : {};

    try {
      trackCommand(commandName, options);
      const result = await fn(...args);
      const duration = Date.now() - startTime;
      trackCommandSuccess(commandName, duration);
      return result;
    } catch (error) {
      const duration = Date.now() - startTime;
      trackCommandError(commandName, error as Error, duration);
      throw error;
    }
  };
}
