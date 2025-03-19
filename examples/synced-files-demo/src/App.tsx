import React, { useEffect, useState } from "react";
import {
  configureSyncEngine,
  configureSyncedFileSystem,
  addFile,
  removeFile,
  getFile,
  getAllFiles,
} from "@tonk/keepsync";
import { FileMetadata } from "@tonk/keepsync";

// Generate a random client ID to distinguish between clients
const CLIENT_ID = Math.random().toString(36).substring(2, 10);

function App() {
  const [files, setFiles] = useState<FileMetadata[]>([]);
  const [isConnected, setIsConnected] = useState(false);
  const [status, setStatus] = useState("Initializing...");
  const [isUploading, setIsUploading] = useState(false);

  // Initialize the sync engine and file system
  useEffect(() => {
    // Configure the sync engine with WebSocket connection
    configureSyncEngine({
      url: "ws://localhost:4080/sync",
      name: `Client-${CLIENT_ID}`,
      onSync: (docId) => {
        setStatus(`Synced document: ${docId}`);
        setIsConnected(true);
        // Refresh file list when sync happens
        loadFiles();
      },
      onError: (error) => {
        console.error("Sync error:", error);
        setStatus(`Sync error: ${error.message}`);
        setIsConnected(false);
      },
    });

    // Configure the synced file system
    configureSyncedFileSystem({
      docId: "shared-files",
      dbName: "demo-file-storage",
      storeName: "file-blobs",
    });

    async function initSync() {
      try {
        // Load initial files
        await loadFiles();

        setStatus("Connected");
        setIsConnected(true);
      } catch (error) {
        console.error("Initialization error:", error);
        setStatus(
          `Initialization error: ${error instanceof Error ? error.message : String(error)}`,
        );
      }
    }

    initSync();
  }, []);

  // Load all files from the synced file system
  const loadFiles = async () => {
    try {
      const allFiles = await getAllFiles();
      if (allFiles) setFiles(allFiles);
    } catch (error) {
      console.error("Error loading files:", error);
    }
  };

  // Handle file upload
  const handleFileUpload = async (
    event: React.ChangeEvent<HTMLInputElement>,
  ) => {
    const fileList = event.target.files;
    if (!fileList || fileList.length === 0) return;

    setIsUploading(true);
    try {
      const file = fileList[0];
      await addFile(file);
      await loadFiles();
      setStatus(`File uploaded: ${file.name}`);
    } catch (error) {
      console.error("Error uploading file:", error);
      setStatus(
        `Upload error: ${error instanceof Error ? error.message : String(error)}`,
      );
    } finally {
      setIsUploading(false);
      // Reset the file input
      event.target.value = "";
    }
  };

  // Handle file download
  const handleDownload = async (fileHash: string, fileName: string) => {
    try {
      const blob = await getFile(fileHash);
      if (!blob) {
        setStatus(`File not found: ${fileName}`);
        return;
      }

      // Create a download link and trigger it
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = fileName;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);

      setStatus(`Downloaded: ${fileName}`);
    } catch (error) {
      console.error("Error downloading file:", error);
      setStatus(
        `Download error: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  };

  // Handle file deletion
  const handleDelete = async (fileHash: string, fileName: string) => {
    try {
      await removeFile(fileHash);
      await loadFiles();
      setStatus(`Deleted: ${fileName}`);
    } catch (error) {
      console.error("Error deleting file:", error);
      setStatus(
        `Delete error: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  };

  // Format file size for display
  const formatFileSize = (bytes: number) => {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  };

  return (
    <div className="container">
      <h1>Synced Files Demo</h1>

      <div className="status-section">
        <div className="status-indicator">
          <div
            className={`status-dot ${isConnected ? "connected" : "disconnected"}`}
          ></div>
          <div className="status-text">{status}</div>
        </div>
        <div className="client-id">Client ID: {CLIENT_ID}</div>
      </div>

      <div className="upload-section">
        <h2>Upload File</h2>
        <div className="upload-form">
          <input
            type="file"
            onChange={handleFileUpload}
            disabled={isUploading || !isConnected}
          />
          {isUploading && <p>Uploading...</p>}
        </div>
      </div>

      <div className="file-list">
        <h2>Files ({files.length})</h2>
        {files.length === 0 ? (
          <p>No files uploaded yet.</p>
        ) : (
          files.map((file) => (
            <div key={file.hash} className="file-item">
              <div className="file-info">
                <span className="file-name">{file.name}</span>
                <span className="file-meta">
                  {formatFileSize(file.size)} •{" "}
                  {new Date(file.lastModified).toLocaleString()}
                </span>
              </div>
              <div className="file-actions">
                <button onClick={() => handleDownload(file.hash, file.name)}>
                  Download
                </button>
                <button
                  className="delete"
                  onClick={() => handleDelete(file.hash, file.name)}
                >
                  Delete
                </button>
              </div>
            </div>
          ))
        )}
      </div>
    </div>
  );
}

export default App;
