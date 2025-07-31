import { sync, ls, mkDir, rm, readDoc, writeDoc, listenToDoc } from './sync.js';
import { DocumentId } from '@automerge/automerge-repo';
export type { DocumentId };

export { ls, sync, mkDir, rm, readDoc, writeDoc, listenToDoc };
export type { SyncOptions } from './sync.js';
