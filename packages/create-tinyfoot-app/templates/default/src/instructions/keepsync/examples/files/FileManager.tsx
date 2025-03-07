import React, { useEffect, useState, useRef } from "react";
import { configureSyncEngine } from "@tonk/keepsync";
import {
  configureSyncedFileSystem,
  addFile,
  removeFile,
  getFile,
  getAllFiles,
} from "@tonk/keepsync";
import { FileMetadata } from "@tonk/keepsync";

// Initialize the sync engine and file system
export function initializeFileSync() {
  // Configure the sync engine
  configureSyncEngine({
    url: "ws://localhost:3030/sync",
    name: "FileManagerApplication",
    dbName: "file_manager_example_db",
    onSync: (docId) => console.log(`Document ${docId} synced`),
    onError: (error) => console.error("Sync error:", error),
  });

  // Configure the synced file system
  configureSyncedFileSystem({
    docId: "shared-files",
    dbName: "file_manager_example_db",
    storeName: "file-blobs",
  });
}

// File item component
const FileItem: React.FC<{
  file: FileMetadata;
  onRemove: () => void;
  onDownload: () => void;
}> = ({ file, onRemove, onDownload }) => {
  // Format file size
  const formatFileSize = (bytes: number): string => {
    if (bytes < 1024) return bytes + " B";
    else if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(2) + " KB";
    else if (bytes < 1024 * 1024 * 1024)
      return (bytes / (1024 * 1024)).toFixed(2) + " MB";
    else return (bytes / (1024 * 1024 * 1024)).toFixed(2) + " GB";
  };

  // Format date
  const formatDate = (timestamp: number): string => {
    return new Date(timestamp).toLocaleString();
  };

  // Get file icon based on type
  const getFileIcon = (type: string): string => {
    if (type.startsWith("image/")) return "🖼️";
    if (type.startsWith("video/")) return "🎬";
    if (type.startsWith("audio/")) return "🎵";
    if (type.startsWith("text/")) return "📄";
    if (type.includes("pdf")) return "📑";
    if (type.includes("word") || type.includes("document")) return "📝";
    if (type.includes("excel") || type.includes("sheet")) return "📊";
    if (type.includes("powerpoint") || type.includes("presentation"))
      return "📽️";
    if (type.includes("zip") || type.includes("compressed")) return "🗜️";
    return "📁";
  };

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        padding: "12px",
        borderBottom: "1px solid #eee",
        backgroundColor: "#f9f9f9",
        borderRadius: "4px",
        marginBottom: "8px",
      }}
    >
      <div style={{ fontSize: "24px", marginRight: "12px" }}>
        {getFileIcon(file.type)}
      </div>
      <div style={{ flex: 1 }}>
        <div style={{ fontWeight: "bold" }}>{file.name}</div>
        <div style={{ fontSize: "12px", color: "#666" }}>
          {formatFileSize(file.size)} • {formatDate(file.lastModified)}
        </div>
      </div>
      <button
        onClick={onDownload}
        style={{
          marginRight: "8px",
          padding: "6px 12px",
          background: "#4caf50",
          color: "white",
          border: "none",
          borderRadius: "4px",
          cursor: "pointer",
        }}
      >
        Download
      </button>
      <button
        onClick={onRemove}
        style={{
          padding: "6px 12px",
          background: "#ff6b6b",
          color: "white",
          border: "none",
          borderRadius: "4px",
          cursor: "pointer",
        }}
      >
        Remove
      </button>
    </div>
  );
};

// FileManager component
export const FileManager: React.FC = () => {
  const [files, setFiles] = useState<FileMetadata[]>([]);
  const [loading, setLoading] = useState<boolean>(true);
  const [uploading, setUploading] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // Load files on component mount
  useEffect(() => {
    // Initialize sync
    initializeFileSync();

    // Load files
    loadFiles();

    // Set up polling to check for updates
    const interval = setInterval(loadFiles, 5000);
    return () => clearInterval(interval);
  }, []);

  // Load all files from the synced file system
  const loadFiles = async () => {
    try {
      const allFiles = await getAllFiles();
      setFiles(allFiles);
      setLoading(false);
    } catch (err) {
      console.error("Error loading files:", err);
      setError("Failed to load files. Please try again.");
      setLoading(false);
    }
  };

  // Handle file upload
  const handleFileUpload = async (
    event: React.ChangeEvent<HTMLInputElement>,
  ) => {
    const selectedFiles = event.target.files;
    if (!selectedFiles || selectedFiles.length === 0) return;

    setUploading(true);
    setError(null);

    try {
      // Upload each file
      for (let i = 0; i < selectedFiles.length; i++) {
        const file = selectedFiles[i];
        await addFile(file);
      }

      // Reload the file list
      await loadFiles();

      // Reset the file input
      if (fileInputRef.current) {
        fileInputRef.current.value = "";
      }
    } catch (err) {
      console.error("Error uploading file:", err);
      setError("Failed to upload file. Please try again.");
    } finally {
      setUploading(false);
    }
  };

  // Handle file removal
  const handleRemoveFile = async (hash: string) => {
    try {
      await removeFile(hash);
      // Update the UI immediately
      setFiles(files.filter((file) => file.hash !== hash));
    } catch (err) {
      console.error("Error removing file:", err);
      setError("Failed to remove file. Please try again.");
    }
  };

  // Handle file download
  const handleDownloadFile = async (file: FileMetadata) => {
    try {
      const blob = await getFile(file.hash);
      if (!blob) {
        setError("File not found. It may have been removed.");
        return;
      }

      // Create a download link
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = file.name;
      document.body.appendChild(a);
      a.click();

      // Clean up
      setTimeout(() => {
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
      }, 100);
    } catch (err) {
      console.error("Error downloading file:", err);
      setError("Failed to download file. Please try again.");
    }
  };

  return (
    <div
      style={{
        maxWidth: "800px",
        margin: "0 auto",
        padding: "20px",
        fontFamily: "Arial, sans-serif",
      }}
    >
      <h1 style={{ textAlign: "center", color: "#333" }}>
        Synced File Manager
      </h1>

      {/* Upload section */}
      <div
        style={{
          marginBottom: "20px",
          padding: "20px",
          backgroundColor: "#f0f0f0",
          borderRadius: "8px",
          textAlign: "center",
        }}
      >
        <h2 style={{ margin: "0 0 15px 0", fontSize: "18px" }}>Upload Files</h2>
        <input
          type="file"
          ref={fileInputRef}
          onChange={handleFileUpload}
          multiple
          style={{ display: "none" }}
          id="file-upload"
        />
        <label
          htmlFor="file-upload"
          style={{
            display: "inline-block",
            padding: "10px 20px",
            backgroundColor: "#4285f4",
            color: "white",
            borderRadius: "4px",
            cursor: "pointer",
            fontWeight: "bold",
          }}
        >
          {uploading ? "Uploading..." : "Select Files to Upload"}
        </label>
        <p style={{ fontSize: "14px", color: "#666", marginTop: "10px" }}>
          Files will be automatically synced with all connected clients
        </p>
      </div>

      {/* Error message */}
      {error && (
        <div
          style={{
            padding: "10px",
            backgroundColor: "#ffebee",
            color: "#c62828",
            borderRadius: "4px",
            marginBottom: "20px",
          }}
        >
          {error}
        </div>
      )}

      {/* Files list */}
      <div>
        <h2 style={{ borderBottom: "1px solid #eee", paddingBottom: "10px" }}>
          Shared Files ({files.length})
        </h2>

        {loading ? (
          <div style={{ textAlign: "center", padding: "20px" }}>
            Loading files...
          </div>
        ) : files.length === 0 ? (
          <div style={{ textAlign: "center", padding: "30px", color: "#999" }}>
            No files uploaded yet. Upload some files to get started!
          </div>
        ) : (
          <div>
            {files.map((file) => (
              <FileItem
                key={file.hash}
                file={file}
                onRemove={() => handleRemoveFile(file.hash)}
                onDownload={() => handleDownloadFile(file)}
              />
            ))}
          </div>
        )}
      </div>

      {/* Sync status */}
      <div
        style={{
          marginTop: "30px",
          padding: "15px",
          background: "#f5f5f5",
          borderRadius: "4px",
          fontSize: "14px",
          color: "#666",
        }}
      >
        <p style={{ margin: "0 0 10px 0" }}>
          <strong>Sync Status:</strong> Active
        </p>
        <p style={{ margin: 0 }}>
          Files are automatically synchronized across all connected clients. Try
          opening this app in multiple browser windows to see real-time updates!
        </p>
      </div>
    </div>
  );
};

// Usage example:
// In your application entry point:
// initializeFileSync();
//
// In your component:
// function App() {
//   return <FileManager />;
// }
