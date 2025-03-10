import { configureSyncEngine } from "@tonk/keepsync";
import {
  configureSyncedFileSystem,
  addFile,
  removeFile,
  getFile,
  getAllFiles,
} from "@tonk/keepsync";
import { FileMetadata } from "@tonk/keepsync";

/**
 * Initialize the sync engine and file system
 */
export function initializeFileSync() {
  // Configure the sync engine
  configureSyncEngine({
    url: "ws://localhost:4080/sync",
    name: "BasicFileManagerExample",
    dbName: "basic_file_manager_db",
    onSync: (docId) => console.log(`Document ${docId} synced`),
    onError: (error) => console.error("Sync error:", error),
  });

  // Configure the synced file system
  configureSyncedFileSystem({
    docId: "basic-files",
    dbName: "basic_file_manager_db",
    storeName: "file-blobs",
  });
}

/**
 * Basic file operations
 */
export const fileOperations = {
  /**
   * Upload a file to the synced file system
   * @param file The file to upload
   * @returns Metadata for the uploaded file
   */
  uploadFile: async (file: File): Promise<FileMetadata> => {
    console.log(`Uploading file: ${file.name} (${file.size} bytes)`);
    return addFile(file);
  },

  /**
   * Delete a file from the synced file system
   * @param hash The hash of the file to delete
   */
  deleteFile: async (hash: string): Promise<void> => {
    console.log(`Deleting file with hash: ${hash}`);
    return removeFile(hash);
  },

  /**
   * Download a file from the synced file system
   * @param hash The hash of the file to download
   * @param filename Optional filename to use for the download
   * @returns The downloaded blob or null if not found
   */
  downloadFile: async (
    hash: string,
    filename?: string,
  ): Promise<Blob | null> => {
    console.log(`Downloading file with hash: ${hash}`);
    const blob = await getFile(hash);

    if (blob && filename) {
      // Create a download link
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = filename;
      document.body.appendChild(a);
      a.click();

      // Clean up
      setTimeout(() => {
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
      }, 100);
    }

    return blob;
  },

  /**
   * List all files in the synced file system
   * @returns Array of file metadata
   */
  listFiles: async (): Promise<FileMetadata[]> => {
    console.log("Listing all files");
    const files = await getAllFiles();
    console.log(`Found ${files.length} files`);
    return files;
  },
};

// Usage example:
/*
// Initialize the sync system
initializeFileSync();

// Upload a file
const fileInput = document.getElementById('fileInput') as HTMLInputElement;
fileInput.addEventListener('change', async (event) => {
  const files = fileInput.files;
  if (files && files.length > 0) {
    const metadata = await fileOperations.uploadFile(files[0]);
    console.log('Uploaded file metadata:', metadata);
  }
});

// List all files
const listFilesButton = document.getElementById('listFiles');
listFilesButton.addEventListener('click', async () => {
  const files = await fileOperations.listFiles();
  
  // Display files in the UI
  const filesList = document.getElementById('filesList');
  filesList.innerHTML = '';
  
  files.forEach(file => {
    const item = document.createElement('div');
    item.textContent = `${file.name} (${file.size} bytes)`;
    
    const downloadButton = document.createElement('button');
    downloadButton.textContent = 'Download';
    downloadButton.onclick = () => fileOperations.downloadFile(file.hash, file.name);
    
    const deleteButton = document.createElement('button');
    deleteButton.textContent = 'Delete';
    deleteButton.onclick = async () => {
      await fileOperations.deleteFile(file.hash);
      listFilesButton.click(); // Refresh the list
    };
    
    item.appendChild(downloadButton);
    item.appendChild(deleteButton);
    filesList.appendChild(item);
  });
});
*/
