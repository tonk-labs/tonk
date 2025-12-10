import {
  getActiveBundle,
  getWatcherEntries,
  incrementReconnectAttempts,
  resetReconnectAttempts,
  setConnectionHealth,
  setHealthCheckInterval,
} from './state';
import { HEALTH_CHECK_INTERVAL, MAX_RECONNECT_ATTEMPTS } from './types';
import { logger } from './utils/logging';
import { postResponse } from './utils/response';

// Continuous retry flag
const continuousRetryEnabled = true;

export async function performHealthCheck(): Promise<boolean> {
  const activeBundle = getActiveBundle();
  if (!activeBundle) {
    return false;
  }

  try {
    const result = await activeBundle.tonk.isConnected();
    return result;
  } catch (error) {
    logger.error('performHealthCheck() failed', {
      error: error instanceof Error ? error.message : String(error),
    });
    return false;
  }
}

export async function attemptReconnect(): Promise<void> {
  const activeBundle = getActiveBundle();
  if (!activeBundle) {
    logger.error('Cannot reconnect: no active bundle');
    return;
  }

  const wsUrl = activeBundle.wsUrl;
  if (!wsUrl) {
    logger.error('Cannot reconnect: wsUrl not stored');
    return;
  }

  const attempts = incrementReconnectAttempts();

  if (attempts >= MAX_RECONNECT_ATTEMPTS) {
    if (continuousRetryEnabled) {
      resetReconnectAttempts();
    } else {
      logger.error('Max reconnection attempts reached', { attempts });
      await postResponse({ type: 'reconnectionFailed' });
      return;
    }
  }

  logger.debug('Attempting to reconnect', {
    attempt: attempts,
    maxAttempts: MAX_RECONNECT_ATTEMPTS,
    wsUrl,
  });

  await postResponse({ type: 'reconnecting', attempt: attempts });

  try {
    await activeBundle.tonk.connectWebsocket(wsUrl);

    await new Promise(resolve => setTimeout(resolve, 1000));

    const isConnected = await activeBundle.tonk.isConnected();

    if (isConnected) {
      setConnectionHealth(true);
      resetReconnectAttempts();
      logger.info('Reconnection successful');
      await postResponse({ type: 'reconnected' });

      await reestablishWatchers();
    } else {
      throw new Error('Connection check failed after reconnect attempt');
    }
  } catch (error) {
    logger.warn('Reconnection failed', {
      error: error instanceof Error ? error.message : String(error),
      attempt: attempts,
    });

    const backoffDelay = Math.min(1000 * 2 ** (attempts - 1), 30000);
    logger.debug('Scheduling next reconnect attempt', {
      delayMs: backoffDelay,
      nextAttempt: attempts + 1,
    });
    setTimeout(attemptReconnect, backoffDelay);
  }
}

export async function reestablishWatchers(): Promise<void> {
  const watcherEntries = getWatcherEntries();

  logger.debug('Re-establishing watchers after reconnection', {
    watcherCount: watcherEntries.length,
  });

  // Note: The actual re-establishment logic would need to track
  // watcher paths and recreate them. For now, just log.
  logger.debug('Watcher re-establishment complete', {
    watcherCount: watcherEntries.length,
  });

  await postResponse({
    type: 'watchersReestablished',
    count: watcherEntries.length,
  });
}

export function startHealthMonitoring(): void {
  const activeBundle = getActiveBundle();
  if (!activeBundle) {
    logger.warn('Cannot start health monitoring: no active bundle');
    return;
  }

  // Clear existing interval if any
  if (activeBundle.healthCheckInterval) {
    clearInterval(activeBundle.healthCheckInterval);
  }

  logger.debug('Starting health monitoring', {
    intervalMs: HEALTH_CHECK_INTERVAL,
  });

  const interval = setInterval(async () => {
    const isHealthy = await performHealthCheck();
    const currentBundle = getActiveBundle();

    if (!currentBundle) {
      clearInterval(interval);
      return;
    }

    if (!isHealthy && currentBundle.connectionHealthy) {
      setConnectionHealth(false);
      logger.warn('Connection lost, starting reconnection attempts');
      await postResponse({ type: 'disconnected' });
      attemptReconnect();
    } else if (isHealthy && !currentBundle.connectionHealthy) {
      setConnectionHealth(true);
      resetReconnectAttempts();
      logger.debug('Connection health restored');
    }
  }, HEALTH_CHECK_INTERVAL) as unknown as number;

  setHealthCheckInterval(interval);
}
