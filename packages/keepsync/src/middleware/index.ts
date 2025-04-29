import {sync, readDoc, writeDoc} from './sync';
import {DocumentId} from '@automerge/automerge-repo';
export type {DocumentId};

export default sync;
export {sync, readDoc, writeDoc};
export type {SyncOptions} from './sync';
