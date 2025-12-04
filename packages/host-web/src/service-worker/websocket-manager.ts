import { log } from './logging';
import {
  HEALTH_CHECK_INTERVAL,
  MAX_RECONNECT_ATTEMPTS,
  CONTINUOUS_RETRY_ENABLED,
} from './constants';
import {
  getTonk,
  getWsUrl,
  getWatchers,
  isConnectionHealthy,
  getReconnectAttempts,
  setConnectionHealthy,
  setReconnectAttempts,
  incrementReconnectAttempts,
  resetReconnectAttempts,
  setHealthCheckInterval,
  clearHealthCheckInterval,
} from './state';
import { postResponse } from './message-utils';

export async function performHealthCheck(): Promise<boolean> {
  const tonkInstance = getTonk();
  if (!tonkInstance) {
    return false;
  }

  try {
    const result = await tonkInstance.tonk.isConnected();
    return result;
  } catch (error) {
    console.error('🏥 [SW] performHealthCheck() ERROR:', error);
    return false;
  }
}

export async function attemptReconnect(): Promise<void> {
  const wsUrl = getWsUrl();
  if (!wsUrl) {
    log('error', 'Cannot reconnect: wsUrl not stored');
    console.error('🔄 [SW] Cannot reconnect: wsUrl not stored');
    return;
  }

  if (getReconnectAttempts() >= MAX_RECONNECT_ATTEMPTS) {
    if (CONTINUOUS_RETRY_ENABLED) {
      resetReconnectAttempts();
    } else {
      log('error', 'Max reconnection attempts reached', {
        attempts: getReconnectAttempts(),
      });
      console.error(
        '🔄 [SW] Max reconnection attempts reached:',
        getReconnectAttempts()
      );
      await postResponse({ type: 'reconnectionFailed' });
      return;
    }
  }

  const tonkInstance = getTonk();
  if (!tonkInstance) {
    log('error', 'Cannot reconnect: tonk not initialized');
    console.error('🔄 [SW] Cannot reconnect: tonk not initialized');
    return;
  }

  const currentAttempt = incrementReconnectAttempts();
  log('info', 'Attempting to reconnect', {
    attempt: currentAttempt,
    maxAttempts: MAX_RECONNECT_ATTEMPTS,
    wsUrl,
  });

  await postResponse({ type: 'reconnecting', attempt: currentAttempt });

  try {
    await tonkInstance.tonk.connectWebsocket(wsUrl);

    await new Promise(resolve => setTimeout(resolve, 1000));

    const isConnected = await tonkInstance.tonk.isConnected();

    if (isConnected) {
      setConnectionHealthy(true);
      resetReconnectAttempts();
      log('info', 'Reconnection successful');
      await postResponse({ type: 'reconnected' });

      await reestablishWatchers();
    } else {
      throw new Error('Connection check failed after reconnect attempt');
    }
  } catch (error) {
    log('error', 'Reconnection failed', {
      error: error instanceof Error ? error.message : String(error),
      attempt: currentAttempt,
    });

    const backoffDelay = Math.min(
      1000 * Math.pow(2, currentAttempt - 1),
      30000
    );
    log('info', 'Scheduling next reconnect attempt', {
      delayMs: backoffDelay,
      nextAttempt: currentAttempt + 1,
    });
    setTimeout(attemptReconnect, backoffDelay);
  }
}

export async function reestablishWatchers(): Promise<void> {
  const watchers = getWatchers();
  log('info', 'Re-establishing watchers after reconnection', {
    watcherCount: watchers.size,
  });

  const tonkInstance = getTonk();
  if (!tonkInstance) {
    log('error', 'Cannot re-establish watchers: tonk not available');
    return;
  }

  const watcherInfo = Array.from(watchers.entries());

  log('info', 'Watcher re-establishment complete', {
    watcherCount: watcherInfo.length,
  });

  await postResponse({
    type: 'watchersReestablished',
    count: watcherInfo.length,
  });
}

export function startHealthMonitoring(): void {
  clearHealthCheckInterval();

  log('info', 'Starting health monitoring', {
    intervalMs: HEALTH_CHECK_INTERVAL,
  });

  const interval = setInterval(async () => {
    const isHealthy = await performHealthCheck();

    if (!isHealthy && isConnectionHealthy()) {
      setConnectionHealthy(false);
      log('error', 'Connection lost, starting reconnection attempts');
      console.error('❌ [SW] Connection lost, starting reconnection attempts');
      await postResponse({ type: 'disconnected' });
      attemptReconnect();
    } else if (isHealthy && !isConnectionHealthy()) {
      setConnectionHealthy(true);
      resetReconnectAttempts();
      log('info', 'Connection health restored');
    }
  }, HEALTH_CHECK_INTERVAL) as unknown as number;

  setHealthCheckInterval(interval);
}

export function stopHealthMonitoring(): void {
  clearHealthCheckInterval();
  log('info', 'Health monitoring stopped');
}
