import * as Automerge from '@automerge/automerge';
import { DocumentId } from './types';
import { Storage } from './storage';

export class SyncManager {
  private ws: WebSocket | null = null;
  private syncStates: Map<DocumentId, Automerge.SyncState> = new Map();
  private documents: Map<DocumentId, Automerge.Doc<any>> = new Map();
  private onSyncCallback?: (docId: DocumentId) => void;
  private onErrorCallback?: (error: Error) => void;

  constructor(
    private websocketUrl: string,
    private storage: Storage,
    onSync?: (docId: DocumentId) => void,
    onError?: (error: Error) => void
  ) {
    this.onSyncCallback = onSync;
    this.onErrorCallback = onError;
  }

  connect(): void {
    this.ws = new WebSocket(this.websocketUrl!);

    this.ws.onmessage = async (event) => {
      try {
        const { docId, changes } = JSON.parse(event.data);
        const doc = this.documents.get(docId);

        if (doc) {
          let syncState = this.syncStates.get(docId) || Automerge.initSyncState();
          const [newDoc, newSyncState, patch] = Automerge.receiveSyncMessage(
            doc,
            syncState,
            new Uint8Array(changes)
          );

          if (patch) {
            this.documents.set(docId, newDoc);
            this.syncStates.set(docId, newSyncState);
            await this.storage.saveDocument(docId, Automerge.save(newDoc));
            this.onSyncCallback?.(docId);
          }
        }
      } catch (error) {
        this.onErrorCallback?.(error as Error);
      }
    }

    this.ws.onerror = (error) => {
      this.onErrorCallback?.(error as unknown as Error);
    }
  }

  async sendChanges(docId: DocumentId): Promise<void> {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      throw new Error('WebSocket connection not open');
    }

    const doc = this.documents.get(docId);
    if (!doc) return;

    let syncState = this.syncStates.get(docId) || Automerge.initSyncState();
    const [newSyncState, message] = Automerge.generateSyncMessage(doc, syncState);

    if (message) {
      this.syncStates.set(docId, newSyncState);
      this.ws.send(JSON.stringify({
        docId,
        changes: Array.from(message)
      }));
    }
  }
}
