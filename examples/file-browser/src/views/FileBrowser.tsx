import React, { useState, useEffect } from "react";
import { useNavigate, useLocation } from "react-router-dom";
import { RefNode } from "@tonk/keepsync";
import { KeepsyncService } from "../services/keepsyncService";
import FileItem from "../components/FileItem";

const FileBrowser: React.FC = () => {
  const navigate = useNavigate();
  const location = useLocation();
  const [currentPath, setCurrentPath] = useState<string>("/");
  const [directoryContents, setDirectoryContents] = useState<RefNode[]>([]);
  const [isLoading, setIsLoading] = useState<boolean>(true);
  const [error, setError] = useState<string | null>(null);
  const [newItemName, setNewItemName] = useState<string>("");
  const [isCreatingFile, setIsCreatingFile] = useState<boolean>(false);
  const [isCreatingDirectory, setIsCreatingDirectory] = useState<boolean>(false);
  const [fileContent, setFileContent] = useState<string>("");

  // Extract path from URL query parameters if present
  useEffect(() => {
    const params = new URLSearchParams(location.search);
    const pathParam = params.get("path");
    if (pathParam) {
      setCurrentPath(pathParam);
    }
  }, [location]);

  // Load directory contents
  useEffect(() => {
    loadDirectory(currentPath);
  }, [currentPath]);

  const loadDirectory = async (path: string) => {
    setIsLoading(true);
    setError(null);
    try {
      const result = await KeepsyncService.listDirectory(path);
      if (result && result.children) {
        // Sort directories first, then files, both alphabetically
        const sorted = [...result.children].sort((a, b) => {
          if (a.type !== b.type) {
            return a.type === "dir" ? -1 : 1;
          }
          return a.name.localeCompare(b.name);
        });
        setDirectoryContents(sorted);
      } else {
        setDirectoryContents([]);
      }
    } catch (err) {
      console.error("Error loading directory:", err);
      setError("Failed to load directory contents");
    } finally {
      setIsLoading(false);
    }
  };

  const handleNavigate = (path: string) => {
    if (path.startsWith("/view")) {
      navigate(path);
    } else {
      setCurrentPath(path);
      navigate(`/?path=${encodeURIComponent(path)}`);
    }
  };

  const handleParentDirectory = () => {
    const parentPath = KeepsyncService.getParentPath(currentPath);
    handleNavigate(parentPath);
  };

  const handleDelete = async (path: string) => {
    if (window.confirm(`Are you sure you want to delete ${path}?`)) {
      try {
        await KeepsyncService.removeItem(path);
        loadDirectory(currentPath);
      } catch (err) {
        console.error("Error deleting item:", err);
        setError("Failed to delete item");
      }
    }
  };

  const handleCreateDirectory = async () => {
    if (!newItemName.trim()) {
      setError("Directory name cannot be empty");
      return;
    }

    try {
      const path = KeepsyncService.joinPath(currentPath, newItemName);
      await KeepsyncService.createDirectory(path);
      setNewItemName("");
      setIsCreatingDirectory(false);
      loadDirectory(currentPath);
    } catch (err) {
      console.error("Error creating directory:", err);
      setError("Failed to create directory");
    }
  };

  const handleCreateFile = async () => {
    if (!newItemName.trim()) {
      setError("File name cannot be empty");
      return;
    }

    try {
      const path = KeepsyncService.joinPath(currentPath, newItemName);
      // Create a proper content object to prevent string serialization issues
      const contentToSave = { content: fileContent };
      await KeepsyncService.writeDocument(path, contentToSave);
      setNewItemName("");
      setFileContent("");
      setIsCreatingFile(false);
      loadDirectory(currentPath);
    } catch (err) {
      console.error("Error creating file:", err);
      setError("Failed to create file");
    }
  };

  return (
    <main className="min-h-screen bg-[#f5f5f7] p-4 md:p-6 lg:p-8">
      <section className="max-w-4xl mx-auto">
        <div className="bg-white rounded-2xl shadow-md p-6">
          {/* Header */}
          <div className="mb-6">
            <h1 className="text-3xl font-medium text-[#1d1d1f]">Files</h1>
            <div className="flex items-center mt-2 text-sm text-[#86868b]">
              <span>Current path: </span>
              <div className="ml-2 font-mono bg-[#f5f5f7] px-2 py-1 rounded-md">
                {currentPath}
              </div>
            </div>
          </div>

          {/* Actions */}
          <div className="flex flex-wrap gap-2 mb-6">
            <button
              onClick={handleParentDirectory}
              disabled={currentPath === "/"}
              className={`px-4 py-2 rounded-full text-sm font-medium transition-all ${
                currentPath === "/"
                  ? "bg-[#f5f5f7] text-[#86868b] cursor-not-allowed"
                  : "bg-[#f5f5f7] text-[#0066cc] hover:bg-[#e5e5ea]"
              }`}
            >
              ↑ Parent Directory
            </button>
            <button
              onClick={() => setIsCreatingDirectory(true)}
              className="px-4 py-2 rounded-full text-sm font-medium bg-[#f5f5f7] text-[#0066cc] hover:bg-[#e5e5ea] transition-all"
            >
              + New Folder
            </button>
            <button
              onClick={() => setIsCreatingFile(true)}
              className="px-4 py-2 rounded-full text-sm font-medium bg-[#f5f5f7] text-[#0066cc] hover:bg-[#e5e5ea] transition-all"
            >
              + New File
            </button>
            <button
              onClick={() => loadDirectory(currentPath)}
              className="px-4 py-2 rounded-full text-sm font-medium bg-[#f5f5f7] text-[#0066cc] hover:bg-[#e5e5ea] transition-all"
            >
              ↻ Refresh
            </button>
          </div>

          {/* Create Directory Form */}
          {isCreatingDirectory && (
            <div className="mb-6 p-6 bg-[#f5f5f7] rounded-xl">
              <h3 className="text-xl font-medium mb-4 text-[#1d1d1f]">New Folder</h3>
              <div className="mb-4">
                <input
                  type="text"
                  value={newItemName}
                  onChange={(e) => setNewItemName(e.target.value)}
                  placeholder="Folder name"
                  className="w-full px-4 py-3 border border-[#d2d2d7] rounded-lg focus:outline-none focus:ring-2 focus:ring-[#0066cc] focus:border-transparent"
                  autoFocus
                />
              </div>
              <div className="flex justify-end gap-2">
                <button
                  onClick={() => {
                    setIsCreatingDirectory(false);
                    setNewItemName("");
                  }}
                  className="px-5 py-2.5 rounded-full text-[#0066cc] font-medium hover:bg-[#e5e5ea] transition-all"
                >
                  Cancel
                </button>
                <button
                  onClick={handleCreateDirectory}
                  className="px-5 py-2.5 rounded-full bg-[#0066cc] text-white font-medium hover:bg-[#004499] transition-all"
                >
                  Create
                </button>
              </div>
            </div>
          )}

          {/* Create File Form */}
          {isCreatingFile && (
            <div className="mb-6 p-6 bg-[#f5f5f7] rounded-xl">
              <h3 className="text-xl font-medium mb-4 text-[#1d1d1f]">New File</h3>
              <div className="mb-4">
                <input
                  type="text"
                  value={newItemName}
                  onChange={(e) => setNewItemName(e.target.value)}
                  placeholder="File name"
                  className="w-full px-4 py-3 border border-[#d2d2d7] rounded-lg focus:outline-none focus:ring-2 focus:ring-[#0066cc] focus:border-transparent"
                  autoFocus
                />
              </div>
              <div className="mb-4">
                <textarea
                  value={fileContent}
                  onChange={(e) => setFileContent(e.target.value)}
                  placeholder="File content"
                  className="w-full px-4 py-3 border border-[#d2d2d7] rounded-lg h-32 focus:outline-none focus:ring-2 focus:ring-[#0066cc] focus:border-transparent"
                />
              </div>
              <div className="flex justify-end gap-2">
                <button
                  onClick={() => {
                    setIsCreatingFile(false);
                    setNewItemName("");
                    setFileContent("");
                  }}
                  className="px-5 py-2.5 rounded-full text-[#0066cc] font-medium hover:bg-[#e5e5ea] transition-all"
                >
                  Cancel
                </button>
                <button
                  onClick={handleCreateFile}
                  className="px-5 py-2.5 rounded-full bg-[#0066cc] text-white font-medium hover:bg-[#004499] transition-all"
                >
                  Create
                </button>
              </div>
            </div>
          )}

          {/* Error Message */}
          {error && (
            <div className="mb-6 p-4 bg-[#fef1f2] text-[#ff3b30] rounded-xl border border-[#ffccd0]">
              <div className="flex items-center">
                <svg className="w-5 h-5 mr-2" fill="currentColor" viewBox="0 0 20 20">
                  <path fillRule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zM8.707 7.293a1 1 0 00-1.414 1.414L8.586 10l-1.293 1.293a1 1 0 101.414 1.414L10 11.414l1.293 1.293a1 1 0 001.414-1.414L11.414 10l1.293-1.293a1 1 0 00-1.414-1.414L10 8.586 8.707 7.293z" clipRule="evenodd" />
                </svg>
                {error}
              </div>
            </div>
          )}

          {/* Directory Contents */}
          <div className="rounded-xl overflow-hidden bg-[#f5f5f7]">
            {isLoading ? (
              <div className="p-8 text-center text-[#86868b] flex justify-center items-center">
                <svg className="animate-spin -ml-1 mr-3 h-5 w-5 text-[#0066cc]" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24">
                  <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4"></circle>
                  <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"></path>
                </svg>
                Loading...
              </div>
            ) : directoryContents.length === 0 ? (
              <div className="p-8 text-center text-[#86868b]">
                <svg className="mx-auto h-12 w-12 text-[#d2d2d7] mb-3" fill="currentColor" viewBox="0 0 24 24">
                  <path d="M20 6h-8l-2-2H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2zm0 12H4V8h16v10z" />
                </svg>
                This folder is empty
              </div>
            ) : (
              <div className="divide-y divide-[#d2d2d7]">
                {directoryContents.map((item) => (
                  <FileItem
                    key={item.name}
                    item={item}
                    currentPath={currentPath}
                    onNavigate={handleNavigate}
                    onDelete={handleDelete}
                  />
                ))}
              </div>
            )}
          </div>
        </div>
      </section>
    </main>
  );
};

export default FileBrowser;
