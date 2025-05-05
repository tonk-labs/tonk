import {sync, ls, mkDir, rm, readDoc, writeDoc} from './sync.js';
import {DocumentId} from '@automerge/automerge-repo';
export type {DocumentId};

export {ls, sync, mkDir, rm, readDoc, writeDoc};
export type {SyncOptions} from './sync.js';
