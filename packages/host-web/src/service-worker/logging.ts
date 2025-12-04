import { DEBUG_LOGGING } from './constants';

export type LogLevel = 'info' | 'warn' | 'error';

export function log(level: LogLevel, message: string, data?: unknown): void {
  if (!DEBUG_LOGGING) return;

  const timestamp = new Date().toISOString();
  const prefix = `[VFS Service Worker ${timestamp}] ${level.toUpperCase()}:`;

  if (data !== undefined) {
    console[level](prefix, message, data);
  } else {
    console[level](prefix, message);
  }
}
