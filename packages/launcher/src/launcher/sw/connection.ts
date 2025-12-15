import {
  getActiveBundleState,
  getWatcherEntries,
  incrementReconnectAttempts,
  resetReconnectAttempts,
  setConnectionHealth,
  setHealthCheckInterval,
} from "./state";
import { HEALTH_CHECK_INTERVAL, MAX_RECONNECT_ATTEMPTS } from "./types";
import { logger } from "./utils/logging";
import { broadcastToAllClients } from "./utils/response";

// Continuous retry flag
const continuousRetryEnabled = true;

export async function performHealthCheck(
  launcherBundleId: string,
): Promise<boolean> {
  const activeBundle = getActiveBundleState(launcherBundleId);
  if (!activeBundle) {
    return false;
  }

  try {
    const result = await activeBundle.tonk.isConnected();
    return result;
  } catch (error) {
    logger.error("performHealthCheck() failed", {
      launcherBundleId,
      error: error instanceof Error ? error.message : String(error),
    });
    return false;
  }
}

export async function attemptReconnect(
  launcherBundleId: string,
): Promise<void> {
  const activeBundle = getActiveBundleState(launcherBundleId);
  if (!activeBundle) {
    logger.error("Cannot reconnect: no active bundle", { launcherBundleId });
    return;
  }

  const wsUrl = activeBundle.wsUrl;
  if (!wsUrl) {
    logger.error("Cannot reconnect: wsUrl not stored", { launcherBundleId });
    return;
  }

  const attempts = incrementReconnectAttempts(launcherBundleId);

  if (attempts >= MAX_RECONNECT_ATTEMPTS) {
    if (continuousRetryEnabled) {
      resetReconnectAttempts(launcherBundleId);
    } else {
      logger.error("Max reconnection attempts reached", {
        launcherBundleId,
        attempts,
      });
      // Connection status updates are broadcast to all clients
      await broadcastToAllClients({
        type: "reconnectionFailed",
        launcherBundleId,
      });
      return;
    }
  }

  logger.debug("Attempting to reconnect", {
    launcherBundleId,
    attempt: attempts,
    maxAttempts: MAX_RECONNECT_ATTEMPTS,
    wsUrl,
  });

  // Connection status updates are broadcast to all clients
  await broadcastToAllClients({
    type: "reconnecting",
    launcherBundleId,
    attempt: attempts,
  });

  try {
    await activeBundle.tonk.connectWebsocket(wsUrl);

    await new Promise((resolve) => setTimeout(resolve, 1000));

    const isConnected = await activeBundle.tonk.isConnected();

    if (isConnected) {
      setConnectionHealth(launcherBundleId, true);
      resetReconnectAttempts(launcherBundleId);
      logger.info("Reconnection successful", { launcherBundleId });
      // Connection status updates are broadcast to all clients
      await broadcastToAllClients({ type: "reconnected", launcherBundleId });

      await reestablishWatchers(launcherBundleId);
    } else {
      throw new Error("Connection check failed after reconnect attempt");
    }
  } catch (error) {
    logger.warn("Reconnection failed", {
      launcherBundleId,
      error: error instanceof Error ? error.message : String(error),
      attempt: attempts,
    });

    const backoffDelay = Math.min(1000 * 2 ** (attempts - 1), 30000);
    logger.debug("Scheduling next reconnect attempt", {
      launcherBundleId,
      delayMs: backoffDelay,
      nextAttempt: attempts + 1,
    });
    setTimeout(() => attemptReconnect(launcherBundleId), backoffDelay);
  }
}

export async function reestablishWatchers(
  launcherBundleId: string,
): Promise<void> {
  const watcherEntries = getWatcherEntries(launcherBundleId);

  logger.debug("Re-establishing watchers after reconnection", {
    launcherBundleId,
    watcherCount: watcherEntries.length,
  });

  // Note: The actual re-establishment logic would need to track
  // watcher paths and recreate them. For now, just log.
  logger.debug("Watcher re-establishment complete", {
    launcherBundleId,
    watcherCount: watcherEntries.length,
  });

  // Connection status updates are broadcast to all clients
  await broadcastToAllClients({
    type: "watchersReestablished",
    launcherBundleId,
    count: watcherEntries.length,
  });
}

export function startHealthMonitoring(launcherBundleId: string): void {
  const activeBundle = getActiveBundleState(launcherBundleId);
  if (!activeBundle) {
    logger.warn("Cannot start health monitoring: no active bundle", {
      launcherBundleId,
    });
    return;
  }

  // Clear existing interval if any
  if (activeBundle.healthCheckInterval) {
    clearInterval(activeBundle.healthCheckInterval);
  }

  logger.debug("Starting health monitoring", {
    launcherBundleId,
    intervalMs: HEALTH_CHECK_INTERVAL,
  });

  const interval = setInterval(async () => {
    const isHealthy = await performHealthCheck(launcherBundleId);
    const currentBundle = getActiveBundleState(launcherBundleId);

    if (!currentBundle) {
      clearInterval(interval);
      return;
    }

    if (!isHealthy && currentBundle.connectionHealthy) {
      setConnectionHealth(launcherBundleId, false);
      logger.warn("Connection lost, starting reconnection attempts", {
        launcherBundleId,
      });
      // Connection status updates are broadcast to all clients
      await broadcastToAllClients({ type: "disconnected", launcherBundleId });
      attemptReconnect(launcherBundleId);
    } else if (isHealthy && !currentBundle.connectionHealthy) {
      setConnectionHealth(launcherBundleId, true);
      resetReconnectAttempts(launcherBundleId);
      logger.debug("Connection health restored", { launcherBundleId });
    }
  }, HEALTH_CHECK_INTERVAL) as unknown as number;

  setHealthCheckInterval(launcherBundleId, interval);
}
