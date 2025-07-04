import {getDeploymentServiceUrl} from '../config/environment.js';

// Lazy-load tonkAuth to get auth token
async function getTonkAuth() {
  const {tonkAuth} = await import('../lib/tonkAuth.js');
  return tonkAuth;
}

/**
 * Fetches the list of servers owned by the authenticated user
 */
export async function fetchUserServers(): Promise<string[]> {
  try {
    const tonkAuth = await getTonkAuth();
    const authToken = await tonkAuth.getAuthToken();
    if (!authToken) {
      throw new Error('Failed to get authentication token');
    }

    const deploymentServiceUrl = getDeploymentServiceUrl();
    const response = await fetch(`${deploymentServiceUrl}/list-user-servers`, {
      method: 'GET',
      headers: {
        'Authorization': `Bearer ${authToken}`,
      },
    });

    const result: any = await response.json();

    if (!response.ok || !result.success) {
      throw new Error(result.error || 'Failed to fetch user servers');
    }

    return result.servers || [];
  } catch (error) {
    throw new Error(`Failed to fetch servers: ${error instanceof Error ? error.message : String(error)}`);
  }
}