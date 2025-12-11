/**
 * Service Worker Logging Module
 *
 * Log levels (in order of severity):
 *   debug < info < warn < error
 *
 * By default, only 'warn' and 'error' are printed.
 *
 * To enable more verbose logging for debugging:
 *   - In browser console: self.SW_LOG_LEVEL = 'debug'
 *   - Or: self.SW_LOG_LEVEL = 'info'
 *
 * To disable all logging:
 *   - self.SW_LOG_LEVEL = 'none'
 */

export type LogLevel = 'debug' | 'info' | 'warn' | 'error' | 'none';

const LOG_LEVELS: Record<LogLevel, number> = {
  debug: 0,
  info: 1,
  warn: 2,
  error: 3,
  none: 4,
};

// Extend ServiceWorkerGlobalScope to include our log level property
declare global {
  interface ServiceWorkerGlobalScope {
    SW_LOG_LEVEL?: LogLevel;
  }
}

// Default log level - only show warnings and errors
const DEFAULT_LOG_LEVEL: LogLevel = 'warn';

/**
 * Get the current log level.
 * Checks self.SW_LOG_LEVEL for runtime override.
 */
function getCurrentLogLevel(): LogLevel {
  const swSelf = self as unknown as ServiceWorkerGlobalScope;
  if (swSelf.SW_LOG_LEVEL && swSelf.SW_LOG_LEVEL in LOG_LEVELS) {
    return swSelf.SW_LOG_LEVEL;
  }
  return DEFAULT_LOG_LEVEL;
}

/**
 * Check if a given level should be logged based on current settings.
 */
function shouldLog(level: LogLevel): boolean {
  const currentLevel = getCurrentLogLevel();
  return LOG_LEVELS[level] >= LOG_LEVELS[currentLevel];
}

/**
 * Format the log prefix with timestamp and level.
 */
function formatPrefix(level: LogLevel): string {
  const timestamp = new Date().toISOString();
  return `[SW ${timestamp}] ${level.toUpperCase()}:`;
}

/**
 * Main logging function.
 *
 * @param level - The log level
 * @param message - The log message
 * @param data - Optional data to log
 */
export function log(level: Exclude<LogLevel, 'none'>, message: string, data?: unknown): void {
  if (!shouldLog(level)) return;

  const prefix = formatPrefix(level);

  // Map log levels to console methods
  const consoleMethod = level === 'debug' ? 'log' : level;

  if (data !== undefined) {
    console[consoleMethod](prefix, message, data);
  } else {
    console[consoleMethod](prefix, message);
  }
}

/**
 * Convenience methods for each log level.
 */
export const logger = {
  debug: (message: string, data?: unknown) => log('debug', message, data),
  info: (message: string, data?: unknown) => log('info', message, data),
  warn: (message: string, data?: unknown) => log('warn', message, data),
  error: (message: string, data?: unknown) => log('error', message, data),

  /**
   * Set the log level at runtime.
   * @param level - The new log level
   */
  setLevel: (level: LogLevel) => {
    const swSelf = self as unknown as ServiceWorkerGlobalScope;
    swSelf.SW_LOG_LEVEL = level;
    // Always print this message so user knows it worked
    console.log(`[SW] Log level set to: ${level}`);
  },

  /**
   * Get the current log level.
   */
  getLevel: (): LogLevel => getCurrentLogLevel(),
};

// Legacy export for backwards compatibility during migration
export const DEBUG_LOGGING = false; // Deprecated - use logger.setLevel('debug') instead
