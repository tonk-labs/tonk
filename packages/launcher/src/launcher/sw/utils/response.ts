import '../types'; // Import for global type declarations
import { log } from './logging';

// Helper to post messages back to the client that sent the request
// targetClient is required - we don't broadcast to avoid cross-tonk contamination
export function postResponse(response: unknown, targetClient: Client): void {
  log('info', 'Posting response to client', {
    type: (response as { type?: string }).type,
    success:
      'success' in (response as object)
        ? (response as { success: boolean }).success
        : 'N/A',
    clientId: targetClient.id,
  });

  targetClient.postMessage(response);
}

// Helper to send response to a specific client by ID
// Used for async callbacks (like watchers) where we stored the client ID
export async function postResponseToClientId(
  response: unknown,
  clientId: string
): Promise<boolean> {
  const swSelf = self as unknown as ServiceWorkerGlobalScope;
  const clients = await swSelf.clients.matchAll();
  const targetClient = clients.find(c => c.id === clientId);

  if (targetClient) {
    log('info', 'Posting response to client by ID', {
      type: (response as { type?: string }).type,
      clientId,
    });
    targetClient.postMessage(response);
    return true;
  } else {
    log('warn', 'Target client not found, may have disconnected', {
      type: (response as { type?: string }).type,
      clientId,
    });
    return false;
  }
}

// Broadcast to all clients - only used for service worker lifecycle events (ready, etc.)
// This should NOT be used for tonk-specific operations
export async function broadcastToAllClients(response: unknown): Promise<void> {
  log('info', 'Broadcasting to all clients', {
    type: (response as { type?: string }).type,
  });

  const swSelf = self as unknown as ServiceWorkerGlobalScope;
  const clients = await swSelf.clients.matchAll();

  clients.forEach((client: Client) => {
    client.postMessage(response);
  });
}

// Convert VFS file data to Response
export async function targetToResponse(target: {
  bytes?: string;
  content: { mime?: string } | string;
}): Promise<Response> {
  if (target.bytes) {
    // target.bytes is a base64 string, decode it to binary for Response
    const binaryString = atob(target.bytes);
    const bytes = new Uint8Array(binaryString.length);
    for (let i = 0; i < binaryString.length; i++) {
      bytes[i] = binaryString.charCodeAt(i);
    }
    return new Response(bytes, {
      headers: {
        'Content-Type':
          (target.content as { mime?: string }).mime ||
          'application/octet-stream',
      },
    });
  } else {
    return new Response(target.content as string, {
      headers: { 'Content-Type': 'application/json' },
    });
  }
}
