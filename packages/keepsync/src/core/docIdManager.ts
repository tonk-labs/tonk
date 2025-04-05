import {DocumentId} from '../engine/types.js';
import {logger} from '../utils/logger.js';

// Declare global window properties for TypeScript
declare global {
  interface Window {
    __TONK_DOC_ID__?: string;
    TONK_DOC_ID?: string;
  }
}

// Check for environment variables or global window variables
function getEnvironmentDocId(): string | null {
  // Check for Node.js environment variable
  if (
    typeof process !== 'undefined' &&
    process.env &&
    process.env.TONK_DOC_ID
  ) {
    return process.env.TONK_DOC_ID;
  }

  // Check for browser window variable (set by preload script)
  if (typeof window !== 'undefined') {
    // Check for the global variable
    if (window.__TONK_DOC_ID__) {
      return window.__TONK_DOC_ID__;
    }

    // Check for the exposed API
    if (window.TONK_DOC_ID) {
      return window.TONK_DOC_ID;
    }

    // Check URL parameters
    try {
      const urlParams = new URLSearchParams(window.location.search);
      const docId = urlParams.get('docId');
      if (docId) {
        return docId;
      }
    } catch (e) {
      // Ignore URL parsing errors
    }
  }

  return null;
}

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
  // Check for environment-provided document ID
  const envDocId = getEnvironmentDocId();
  if (envDocId) {
    logger.debug(`Using environment-provided document ID: ${envDocId}`);
    return envDocId as DocumentId;
  }

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
