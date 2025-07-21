import { useState } from "react";
import { Upload, AlertCircle, CheckCircle, Loader2 } from "lucide-react";
import { ls, readDoc } from "@tonk/keepsync";

interface TransferProgress {
  total: number;
  completed: number;
  current?: string;
  errors: string[];
}

export function TransferStores() {
  const [remoteUrl, setRemoteUrl] = useState("");
  const [isTransferring, setIsTransferring] = useState(false);
  const [progress, setProgress] = useState<TransferProgress | null>(null);
  const [isComplete, setIsComplete] = useState(false);

  const validateUrl = (url: string): boolean => {
    try {
      const parsed = new URL(url);
      return parsed.hostname.endsWith(".app.tonk.xyz");
    } catch {
      return false;
    }
  };

  const getAllDocuments = async (path = ""): Promise<string[]> => {
    const documents: string[] = [];

    try {
      console.debug(`Scanning path: ${path || "root"}`);

      const node = await ls(path);
      if (!node || !node.children) {
        console.debug(`No children found at path: ${path || "root"}`);
        return documents;
      }

      console.debug(
        `Found ${node.children.length} children at path: ${path || "root"}`,
      );

      for (const child of node.children) {
        const fullPath = path ? `${path}/${child.name}` : child.name;

        console.debug(`Processing child: ${fullPath} (type: ${child.type})`);

        if (child.type === "doc") {
          try {
            console.debug(`Attempting to read document: ${fullPath}`);
            const content = await readDoc(fullPath);
            if (content !== undefined) {
              documents.push(fullPath);
              console.debug(`Successfully read document: ${fullPath}`);
            } else {
              console.warn(`Document exists but has no content: ${fullPath}`);
            }
          } catch (error) {
            console.error(`Failed to read document: ${fullPath}`, error);
            // Check if this is a WASM error that might crash the app
            if (
              error instanceof Error &&
              error.message.includes("unreachable")
            ) {
              console.error(
                `WASM unreachable error detected for document: ${fullPath}. This document may be corrupted.`,
              );
            }
            // Continue processing other documents instead of failing completely
          }
        } else if (child.type === "dir") {
          try {
            console.debug(`Recursing into directory: ${fullPath}`);
            const subDocs = await getAllDocuments(fullPath);
            documents.push(...subDocs);
            console.debug(
              `Found ${subDocs.length} documents in directory: ${fullPath}`,
            );
          } catch (error) {
            console.error(`Failed to process directory: ${fullPath}`, error);
            // Continue processing other directories
          }
        }
      }
    } catch (error) {
      console.error(
        `Could not list documents at path: ${path || "root"}`,
        error,
      );
    }

    console.debug(
      `Total documents found at path ${path || "root"}: ${documents.length}`,
    );
    return documents;
  };

  const uploadDocument = async (
    path: string,
    remoteUrl: string,
  ): Promise<void> => {
    try {
      console.debug(`Starting upload for document: ${path}`);

      const content = await readDoc(path);
      if (content === undefined) {
        throw new Error(`Document not found or has no content: ${path}`);
      }

      console.debug(
        `Document content loaded for: ${path}, size: ${JSON.stringify(content).length} bytes`,
      );

      const uploadUrl = `${remoteUrl}/keepsync/docs/${encodeURIComponent(path)}`;
      console.debug(`Upload URL: ${uploadUrl}`);

      // Create abort controller for timeout (fallback for browsers without AbortSignal.timeout)
      const controller = new AbortController();
      const timeoutId = setTimeout(() => controller.abort(), 30000); // 30 second timeout

      const response = await fetch(uploadUrl, {
        method: "PUT",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify(content),
        signal: controller.signal,
      });

      clearTimeout(timeoutId);

      if (!response.ok) {
        const errorText = await response.text().catch(() => "Unknown error");
        throw new Error(
          `HTTP ${response.status} ${response.statusText}: ${errorText}`,
        );
      }

      console.debug(`Successfully uploaded document: ${path}`);
    } catch (error) {
      console.error(`Upload failed for document: ${path}`, error);

      // Provide more specific error messages based on error type
      if (error instanceof TypeError && error.message.includes("fetch")) {
        throw new Error(
          `Network error uploading ${path}: Check if the remote server is accessible and the URL is correct`,
        );
      } else if (error instanceof Error && error.name === "AbortError") {
        throw new Error(
          `Upload timeout for ${path}: The server took too long to respond`,
        );
      } else if (
        error instanceof Error &&
        error.message.includes("unreachable")
      ) {
        throw new Error(
          `Document corrupted ${path}: Cannot read document due to data corruption`,
        );
      } else {
        // Re-throw the original error with additional context
        throw new Error(
          `Failed to upload ${path}: ${error instanceof Error ? error.message : "Unknown error"}`,
        );
      }
    }
  };

  const handleTransfer = async () => {
    if (!validateUrl(remoteUrl)) {
      alert("Please enter a valid remote URL (must end with .app.tonk.xyz)");
      return;
    }

    setIsTransferring(true);
    setIsComplete(false);
    setProgress({ total: 0, completed: 0, errors: [] });

    try {
      console.log("Starting document discovery...");
      const documents = await getAllDocuments();
      console.log(`Found ${documents.length} documents to transfer`);

      if (documents.length === 0) {
        alert(
          "No documents found in local keepsync store. This could be due to:\n- Empty store\n- All documents are corrupted\n- Storage initialization issues",
        );
        return;
      }

      setProgress((prev) =>
        prev ? { ...prev, total: documents.length } : null,
      );

      console.log(
        `Starting upload of ${documents.length} documents to ${remoteUrl}`,
      );

      for (let i = 0; i < documents.length; i++) {
        const docPath = documents[i];

        setProgress((prev) =>
          prev
            ? {
                ...prev,
                current: docPath,
                completed: i,
              }
            : null,
        );

        try {
          await uploadDocument(docPath, remoteUrl);
          console.log(`Successfully uploaded: ${docPath}`);
        } catch (error) {
          const errorMsg =
            error instanceof Error ? error.message : "Unknown error";
          console.error(`Upload failed for ${docPath}:`, errorMsg);

          setProgress((prev) =>
            prev
              ? {
                  ...prev,
                  errors: [...prev.errors, errorMsg],
                }
              : null,
          );
        }
      }

      setProgress((prev) =>
        prev
          ? { ...prev, completed: documents.length, current: undefined }
          : null,
      );
      setIsComplete(true);

      const finalProgress = progress;
      if (finalProgress) {
        const successCount =
          finalProgress.completed - finalProgress.errors.length;
        console.log(
          `Transfer complete: ${successCount}/${finalProgress.total} documents uploaded successfully`,
        );
      }
    } catch (error) {
      const errorMsg =
        error instanceof Error ? error.message : "Unknown error occurred";
      console.error("Transfer failed:", errorMsg);

      setProgress((prev) =>
        prev
          ? {
              ...prev,
              errors: [...prev.errors, `Transfer failed: ${errorMsg}`],
            }
          : null,
      );
    } finally {
      setIsTransferring(false);
    }
  };

  const resetTransfer = () => {
    setProgress(null);
    setIsComplete(false);
    setRemoteUrl("");
  };

  return (
    <div className="min-h-screen bg-gray-50 py-12 px-4 sm:px-6 lg:px-8">
      <div className="max-w-md mx-auto">
        <div className="text-center mb-8">
          <Upload className="mx-auto h-12 w-12 text-blue-600" />
          <h1 className="mt-4 text-3xl font-bold text-gray-900">
            Transfer Stores
          </h1>
          <p className="mt-2 text-gray-600">
            Upload your local keepsync documents to a remote server
          </p>
        </div>

        <div className="bg-white shadow rounded-lg p-6">
          {!isTransferring && !isComplete && (
            <div className="space-y-4">
              <div>
                <label
                  htmlFor="remote-url"
                  className="block text-sm font-medium text-gray-700"
                >
                  Remote Server URL
                </label>
                <div className="mt-1">
                  <input
                    type="url"
                    id="remote-url"
                    value={remoteUrl}
                    onChange={(e) => setRemoteUrl(e.target.value)}
                    placeholder="https://your-app.app.tonk.xyz"
                    className="block w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500"
                  />
                </div>
                <p className="mt-1 text-xs text-gray-500">
                  Must be a *.app.tonk.xyz domain
                </p>
              </div>

              <button
                onClick={handleTransfer}
                disabled={!remoteUrl || !validateUrl(remoteUrl)}
                className="w-full flex justify-center py-2 px-4 border border-transparent rounded-md shadow-sm text-sm font-medium text-white bg-blue-600 hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500 disabled:bg-gray-300 disabled:cursor-not-allowed"
              >
                <Upload className="w-4 h-4 mr-2" />
                Start Transfer
              </button>
            </div>
          )}

          {isTransferring && progress && (
            <div className="space-y-4">
              <div className="flex items-center">
                <Loader2 className="animate-spin h-5 w-5 text-blue-600 mr-2" />
                <span className="text-sm font-medium text-gray-900">
                  Transferring documents...
                </span>
              </div>

              <div className="w-full bg-gray-200 rounded-full h-2">
                <div
                  className="bg-blue-600 h-2 rounded-full transition-all duration-300"
                  style={{
                    width: `${(progress.completed / progress.total) * 100}%`,
                  }}
                />
              </div>

              <div className="text-sm text-gray-600">
                {progress.completed} of {progress.total} documents uploaded
              </div>

              {progress.current && (
                <div className="text-xs text-gray-500 truncate">
                  Current: {progress.current}
                </div>
              )}

              {progress.errors.length > 0 && (
                <div className="mt-4">
                  <div className="flex items-center text-red-600 mb-2">
                    <AlertCircle className="h-4 w-4 mr-1" />
                    <span className="text-sm font-medium">
                      Errors ({progress.errors.length})
                    </span>
                  </div>
                  <div className="max-h-32 overflow-y-auto text-xs text-red-600 space-y-1">
                    {progress.errors.map((error, index) => (
                      <div key={index} className="break-words">
                        {error}
                      </div>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}

          {isComplete && progress && (
            <div className="space-y-4">
              <div className="flex items-center text-green-600">
                <CheckCircle className="h-5 w-5 mr-2" />
                <span className="text-sm font-medium">Transfer Complete!</span>
              </div>

              <div className="text-sm text-gray-600">
                Successfully uploaded{" "}
                {progress.completed - progress.errors.length} of{" "}
                {progress.total} documents
              </div>

              {progress.errors.length > 0 && (
                <div className="mt-4">
                  <div className="flex items-center text-red-600 mb-2">
                    <AlertCircle className="h-4 w-4 mr-1" />
                    <span className="text-sm font-medium">
                      {progress.errors.length} documents failed to upload
                    </span>
                  </div>
                  <div className="max-h-32 overflow-y-auto text-xs text-red-600 space-y-1">
                    {progress.errors.map((error, index) => (
                      <div key={index} className="break-words">
                        {error}
                      </div>
                    ))}
                  </div>
                </div>
              )}

              <button
                onClick={resetTransfer}
                className="w-full flex justify-center py-2 px-4 border border-gray-300 rounded-md shadow-sm text-sm font-medium text-gray-700 bg-white hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500"
              >
                Transfer More Documents
              </button>
            </div>
          )}
        </div>

        <div className="mt-6 text-center">
          <p className="text-xs text-gray-500">
            This utility reads all documents from your local keepsync store and
            uploads them to the specified remote server.
          </p>
        </div>
      </div>
    </div>
  );
}
