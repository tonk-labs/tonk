import {PostHog} from 'posthog-node';
import os from 'os';

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

// Singleton PostHog instance
let posthog: PostHog | null = null;

/**
 * Initialize PostHog analytics
 */
export function initAnalytics(): PostHog {
  if (!posthog) {
    posthog = new PostHog('phc_8w3fpFuLkY7Agheficdo7GKPpg4XlXg4irucXhhXXTw', {
      host: 'https://eu.i.posthog.com',
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
    const {machineIdSync} = require('node-machine-id');
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
  additionalProps: Record<string, any> = {},
): void {
  try {
    const analytics = initAnalytics();
    const machineId = getMachineId();

    analytics.capture({
      distinctId: machineId,
      event: 'command_executed',
      properties: {
        command: commandName,
        options,
        platform: os.platform(),
        arch: os.arch(),
        nodeVersion: process.version,
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
  additionalProps: Record<string, any> = {},
): void {
  try {
    const analytics = initAnalytics();
    const machineId = getMachineId();

    analytics.capture({
      distinctId: machineId,
      event: 'command_success',
      properties: {
        command: commandName,
        duration_ms: duration,
        platform: os.platform(),
        arch: os.arch(),
        nodeVersion: process.version,
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
  additionalProps: Record<string, any> = {},
): void {
  try {
    const analytics = initAnalytics();
    const machineId = getMachineId();

    analytics.capture({
      distinctId: machineId,
      event: 'command_error',
      properties: {
        command: commandName,
        error: error instanceof Error ? error.message : error,
        error_type: error instanceof Error ? error.constructor.name : 'string',
        duration_ms: duration,
        platform: os.platform(),
        arch: os.arch(),
        nodeVersion: process.version,
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
  optionsExtractor?: (...args: T) => Record<string, any>,
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
