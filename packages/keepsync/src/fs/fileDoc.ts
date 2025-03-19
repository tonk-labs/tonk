import * as Automerge from '@automerge/automerge';
import {FileManager} from './fileManager';
import {FileMetadata} from './types';

/**
 * Document structure for storing file metadata in Automerge
 */
export interface FileDoc extends Record<string, unknown> {
  /**
   * Array of file metadata objects
   */
  files: FileMetadata[];
}

export class AutomergeFileManager extends FileManager {
  /**
   * Update an Automerge document with file metadata
   */
  updateAutomergeDoc<T extends FileDoc>(
    doc: Automerge.Doc<T>,
    metadata: FileMetadata,
  ): Automerge.Doc<T> {
    return Automerge.change(doc, `Add file: ${metadata.name}`, doc => {
      if (!doc.files) {
        doc.files = [] as any;
      }

      // Check if file already exists
      const existingIndex = doc.files.findIndex(
        (f: FileMetadata) => f.hash === metadata.hash,
      );

      if (existingIndex >= 0) {
        // Update existing file
        doc.files[existingIndex] = metadata as any;
      } else {
        // Add new file
        doc.files.push(metadata as any);
      }
    });
  }

  /**
   * Remove a file from an Automerge document
   */
  removeFileFromAutomergeDoc<T extends FileDoc>(
    doc: Automerge.Doc<T>,
    hash: string,
  ): Automerge.Doc<T> {
    return Automerge.change(doc, `Remove file with hash: ${hash}`, doc => {
      if (!doc.files) {
        return;
      }

      const index = doc.files.findIndex((f: FileMetadata) => f.hash === hash);
      if (index >= 0) {
        doc.files.splice(index, 1);
      }
    });
  }

  /**
   * Get all missing blobs from an Automerge document
   */
  async getMissingBlobs<T extends FileDoc>(
    doc: Automerge.Doc<T>,
  ): Promise<string[]> {
    if (!doc.files || doc.files.length === 0) {
      return [];
    }

    const missing: string[] = [];

    for (const file of doc.files) {
      const exists = await this.hasBlob(file.hash);
      if (!exists) {
        missing.push(file.hash);
      }
    }

    return missing;
  }
}

/**
 * Helper functions
 */
export async function createNewFileDoc(): Promise<Automerge.Doc<FileDoc>> {
  return Automerge.from<FileDoc>({files: []});
}
