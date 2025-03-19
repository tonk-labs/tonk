import {ClientJoinedMessage, SyncMessage} from '../types';

export class MessageProtocol {
  /**
   * Create a client joined message
   */
  static createClientJoinedMessage(clientId: string): ClientJoinedMessage {
    return {
      type: 'client_joined',
      clientId,
      timestamp: Date.now(),
    };
  }

  /**
   * Create a sync message
   */
  static createSyncMessage(docId: string, changes: Uint8Array): SyncMessage {
    return {
      docId,
      changes: Array.from(changes),
    };
  }
}
