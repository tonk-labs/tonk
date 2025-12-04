import { log } from './logging';
import type { VFSWorkerResponse } from './types';

// Helper to post messages back to all clients
export async function postResponse(response: VFSWorkerResponse): Promise<void> {
  log('info', 'Posting response to main thread', {
    type: response.type,
    success: 'success' in response ? response.success : 'N/A',
  });

  // Get all clients and post message to each
  const clients = await (self as unknown as ServiceWorkerGlobalScope).clients.matchAll();

  clients.forEach(client => {
    client.postMessage(response);
  });
}

// Helper to post response with ID matching
export async function postResponseWithId(
  type: string,
  id: string,
  success: boolean,
  data?: unknown,
  error?: string
): Promise<void> {
  const response: Record<string, unknown> = {
    type,
    id,
    success,
  };

  if (data !== undefined) {
    response.data = data;
  }

  if (error !== undefined) {
    response.error = error;
  }

  await postResponse(response as VFSWorkerResponse);
}
