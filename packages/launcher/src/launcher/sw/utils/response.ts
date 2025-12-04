import '../types'; // Import for global type declarations
import { log } from './logging';

// Helper to post messages back to main thread
export async function postResponse(response: unknown) {
  log('info', 'Posting response to main thread', {
    type: (response as { type?: string }).type,
    success: 'success' in (response as object) ? (response as { success: boolean }).success : 'N/A',
  });

  // Get all clients and post message to each
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
        'Content-Type': (target.content as { mime?: string }).mime || 'application/octet-stream',
      },
    });
  } else {
    return new Response(target.content as string, {
      headers: { 'Content-Type': 'application/json' },
    });
  }
}
