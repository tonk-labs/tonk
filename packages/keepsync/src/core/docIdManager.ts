import {DocumentId} from '../engine/types';
import {logger} from '../utils/logger';

// Store for document ID mappings and prefixes
let docIdPrefix: string = '';
let docIdMappings: Record<string, DocumentId> = {};

/**
 * Sets a global prefix for all document IDs
 * This allows for namespace isolation between different instances
 *
 * @param prefix The prefix to add to all document IDs
 */
export function setDocIdPrefix(prefix: string): void {
  docIdPrefix = prefix;
  logger.debug(`Document ID prefix set to: ${prefix}`);
}

/**
 * Gets the current document ID prefix
 *
 * @returns The current document ID prefix
 */
export function getDocIdPrefix(): string {
  return docIdPrefix;
}

/**
 * Maps a logical document ID to an actual document ID
 * This allows for dynamic remapping of document IDs
 *
 * @param logicalId The logical document ID used in application code
 * @param actualId The actual document ID to use for storage and sync
 */
export function mapDocId(logicalId: string, actualId: DocumentId): void {
  docIdMappings[logicalId] = actualId;
  logger.debug(`Mapped document ID: ${logicalId} → ${actualId}`);
}

/**
 * Resolves a logical document ID to its actual document ID
 * If no mapping exists, applies the prefix to the logical ID
 *
 * @param logicalId The logical document ID to resolve
 * @returns The actual document ID to use
 */
export function resolveDocId(logicalId: string): DocumentId {
  // If there's a specific mapping for this ID, use it
  if (docIdMappings[logicalId]) {
    return docIdMappings[logicalId];
  }

  // Otherwise, apply the prefix if one exists
  if (docIdPrefix) {
    return `${docIdPrefix}:${logicalId}` as DocumentId;
  }

  // If no prefix or mapping, use the logical ID directly
  return logicalId as DocumentId;
}

/**
 * Clears all document ID mappings
 */
export function clearDocIdMappings(): void {
  docIdMappings = {};
  logger.debug('Document ID mappings cleared');
}
