import * as Automerge from '@automerge/automerge';
import {vi, describe, it, expect, beforeEach} from 'vitest';
import {
  AutomergeFileManager,
  createNewFileDoc,
  FileDoc,
} from '../../src/fs/fileDoc';
import {FileMetadata} from '../../src/fs/types';

describe('AutomergeFileManager', () => {
  let fileManager: AutomergeFileManager;
  let doc: Automerge.Doc<FileDoc>;

  beforeEach(async () => {
    // Create a new file manager with mocked methods
    fileManager = new AutomergeFileManager();
    // Mock the hasBlob method
    fileManager.hasBlob = vi.fn();

    // Create a fresh document for each test
    doc = await createNewFileDoc();
  });

  describe('updateAutomergeDoc', () => {
    it('should add a new file to the document', () => {
      // Create sample file metadata
      const metadata: FileMetadata = {
        name: 'test.txt',
        hash: 'abc123',
        size: 100,
        type: 'text/plain',
        lastModified: Date.now(),
      };

      // Update the document
      const updatedDoc = fileManager.updateAutomergeDoc(doc, metadata);

      // Check that the file was added
      expect(updatedDoc.files.length).toBe(1);
      expect(updatedDoc.files[0].hash).toBe('abc123');
      expect(updatedDoc.files[0].name).toBe('test.txt');
    });

    it('should update an existing file in the document', () => {
      // Create initial file metadata
      const initialMetadata: FileMetadata = {
        name: 'test.txt',
        hash: 'abc123',
        size: 100,
        type: 'text/plain',
        lastModified: Date.now(),
      };

      // Add the initial file
      let updatedDoc = fileManager.updateAutomergeDoc(doc, initialMetadata);

      // Create updated metadata with the same hash but different properties
      const updatedMetadata: FileMetadata = {
        name: 'test_updated.txt',
        hash: 'abc123',
        size: 200,
        type: 'text/plain',
        lastModified: Date.now(),
      };

      // Update the document with the new metadata
      updatedDoc = fileManager.updateAutomergeDoc(updatedDoc, updatedMetadata);

      // Check that the file was updated, not added
      expect(updatedDoc.files.length).toBe(1);
      expect(updatedDoc.files[0].hash).toBe('abc123');
      expect(updatedDoc.files[0].name).toBe('test_updated.txt');
      expect(updatedDoc.files[0].size).toBe(200);
    });
  });

  describe('removeFileFromAutomergeDoc', () => {
    it('should remove a file from the document', () => {
      // Create sample file metadata
      const metadata: FileMetadata = {
        name: 'test.txt',
        hash: 'abc123',
        size: 100,
        type: 'text/plain',
        lastModified: Date.now(),
      };

      // Add the file to the document
      let updatedDoc = fileManager.updateAutomergeDoc(doc, metadata);

      // Verify the file was added
      expect(updatedDoc.files.length).toBe(1);

      // Remove the file
      updatedDoc = fileManager.removeFileFromAutomergeDoc(updatedDoc, 'abc123');

      // Check that the file was removed
      expect(updatedDoc.files.length).toBe(0);
    });

    it('should do nothing if the file does not exist', () => {
      // Create sample file metadata
      const metadata: FileMetadata = {
        name: 'test.txt',
        hash: 'abc123',
        size: 100,
        type: 'text/plain',
        lastModified: Date.now(),
      };

      // Add the file to the document
      let updatedDoc = fileManager.updateAutomergeDoc(doc, metadata);

      // Try to remove a non-existent file
      updatedDoc = fileManager.removeFileFromAutomergeDoc(
        updatedDoc,
        'nonexistent',
      );

      // Check that the document is unchanged
      expect(updatedDoc.files.length).toBe(1);
      expect(updatedDoc.files[0].hash).toBe('abc123');
    });

    it('should handle empty files array', () => {
      // Try to remove a file from an empty document
      const updatedDoc = fileManager.removeFileFromAutomergeDoc(doc, 'abc123');

      // Check that no errors occurred
      expect(updatedDoc.files.length).toBe(0);
    });
  });

  describe('getMissingBlobs', () => {
    it('should return empty array for empty document', async () => {
      const missing = await fileManager.getMissingBlobs(doc);
      expect(missing).toEqual([]);
    });

    it('should return hashes of missing blobs', async () => {
      // Mock hasBlob to return false for specific hashes
      (fileManager.hasBlob as any).mockImplementation(async (hash: string) => {
        return hash !== 'missing1' && hash !== 'missing2';
      });

      // Add files to the document
      let updatedDoc = doc;
      const files = [
        {
          name: 'file1.txt',
          hash: 'existing',
          size: 100,
          type: 'text/plain',
          lastModified: Date.now(),
        },
        {
          name: 'file2.txt',
          hash: 'missing1',
          size: 200,
          type: 'text/plain',
          lastModified: Date.now(),
        },
        {
          name: 'file3.txt',
          hash: 'missing2',
          size: 300,
          type: 'text/plain',
          lastModified: Date.now(),
        },
      ];

      for (const file of files) {
        updatedDoc = fileManager.updateAutomergeDoc(updatedDoc, file);
      }

      // Get missing blobs
      const missing = await fileManager.getMissingBlobs(updatedDoc);

      // Check that the correct hashes are returned
      expect(missing).toEqual(['missing1', 'missing2']);
      expect(fileManager.hasBlob).toHaveBeenCalledTimes(3);
    });

    it('should return empty array if all blobs exist', async () => {
      // Mock hasBlob to always return true
      (fileManager.hasBlob as any).mockResolvedValue(true);

      // Add a file to the document
      const updatedDoc = fileManager.updateAutomergeDoc(doc, {
        name: 'file.txt',
        hash: 'existing',
        size: 100,
        type: 'text/plain',
        lastModified: Date.now(),
      });

      // Get missing blobs
      const missing = await fileManager.getMissingBlobs(updatedDoc);

      // Check that no hashes are returned
      expect(missing).toEqual([]);
      expect(fileManager.hasBlob).toHaveBeenCalledTimes(1);
    });
  });

  describe('createNewFileDoc', () => {
    it('should create a new document with empty files array', async () => {
      const newDoc = await createNewFileDoc();
      expect(newDoc.files).toEqual([]);
    });
  });
});
