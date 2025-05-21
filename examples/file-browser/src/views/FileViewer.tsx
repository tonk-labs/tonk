import React, { useState, useEffect } from "react";
import { useNavigate, useLocation } from "react-router-dom";
import { KeepsyncService } from "../services/keepsyncService";

const FileViewer: React.FC = () => {
  const navigate = useNavigate();
  const location = useLocation();
  const [filePath, setFilePath] = useState<string>("");
  const [fileContent, setFileContent] = useState<any>(null);
  const [isEditing, setIsEditing] = useState<boolean>(false);
  const [editedContent, setEditedContent] = useState<string>("");
  const [isLoading, setIsLoading] = useState<boolean>(true);
  const [error, setError] = useState<string | null>(null);

  // Extract file path from URL query parameters
  useEffect(() => {
    const params = new URLSearchParams(location.search);
    const pathParam = params.get("path");
    if (pathParam) {
      setFilePath(pathParam);
    } else {
      navigate("/");
    }
  }, [location, navigate]);

  // Load file content
  useEffect(() => {
    if (filePath) {
      loadFile();
    }
  }, [filePath]);

  const loadFile = async () => {
    setIsLoading(true);
    setError(null);
    try {
      const content = await KeepsyncService.readDocument<any>(filePath);
      setFileContent(content);
      
      // Handle our content object format
      if (content && typeof content === 'object' && 'content' in content) {
        if (typeof content.content === 'string') {
          setEditedContent(content.content);
        } else {
          // If content.content is not a string, stringify it
          setEditedContent(JSON.stringify(content.content, null, 2));
        }
      }
      // Handle legacy string content
      else if (typeof content === "string") {
        setEditedContent(content);
      } 
      // Handle other objects
      else if (content !== null && content !== undefined) {
        setEditedContent(JSON.stringify(content, null, 2));
      } 
      // Handle empty content
      else {
        setEditedContent("");
      }
    } catch (err) {
      console.error("Error loading file:", err);
      setError("Failed to load file content");
    } finally {
      setIsLoading(false);
    }
  };

  const handleSave = async () => {
    setIsLoading(true);
    setError(null);
    try {
      let contentToSave: any;
      
      // Check if the edited content is JSON
      if (editedContent.trim().startsWith("{") || editedContent.trim().startsWith("[")) {
        try {
          // Try to parse as JSON
          const parsedContent = JSON.parse(editedContent);
          
          // If it's our content object format, preserve it
          if (parsedContent && typeof parsedContent === 'object' && 'content' in parsedContent) {
            contentToSave = parsedContent;
          } else {
            // Otherwise, wrap it in our content object
            contentToSave = { content: editedContent };
          }
        } catch (e) {
          // If parsing fails, save as string in our content object
          console.warn("Content couldn't be parsed as JSON, saving as string");
          contentToSave = { content: editedContent };
        }
      } else {
        // Plain text, save in our content object
        contentToSave = { content: editedContent };
      }
      
      await KeepsyncService.writeDocument(filePath, contentToSave);
      setFileContent(contentToSave);
      setIsEditing(false);
      loadFile(); // Reload to ensure we have the latest content
    } catch (err) {
      console.error("Error saving file:", err);
      setError("Failed to save file");
    } finally {
      setIsLoading(false);
    }
  };

  const handleDelete = async () => {
    if (window.confirm(`Are you sure you want to delete ${filePath}?`)) {
      try {
        await KeepsyncService.removeItem(filePath);
        // Navigate back to the parent directory
        const parentPath = KeepsyncService.getParentPath(filePath);
        navigate(`/?path=${encodeURIComponent(parentPath)}`);
      } catch (err) {
        console.error("Error deleting file:", err);
        setError("Failed to delete file");
      }
    }
  };

  const handleBack = () => {
    const parentPath = KeepsyncService.getParentPath(filePath);
    navigate(`/?path=${encodeURIComponent(parentPath)}`);
  };

  const renderContent = () => {
    if (fileContent === null || fileContent === undefined) {
      return <div className="text-[#86868b] italic">Empty file</div>;
    }

    // Check if it's our content object format
    if (fileContent && typeof fileContent === 'object' && 'content' in fileContent) {
      return <pre className="whitespace-pre-wrap">{fileContent.content}</pre>;
    }

    // Handle legacy string content
    if (typeof fileContent === "string") {
      return <pre className="whitespace-pre-wrap">{fileContent}</pre>;
    }

    // For other objects and arrays, render as formatted JSON
    return (
      <pre className="whitespace-pre-wrap">
        {JSON.stringify(fileContent, null, 2)}
      </pre>
    );
  };

  return (
    <main className="min-h-screen bg-[#f5f5f7] p-4 md:p-6 lg:p-8">
      <section className="max-w-4xl mx-auto">
        <div className="bg-white rounded-2xl shadow-md p-6">
          {/* Header */}
          <div className="mb-6">
            <h1 className="text-3xl font-medium text-[#1d1d1f]">
              {filePath ? KeepsyncService.getFileName(filePath) : "File"}
            </h1>
            <div className="flex items-center mt-2 text-sm text-[#86868b]">
              <span>Location: </span>
              <div className="ml-2 font-mono bg-[#f5f5f7] px-2 py-1 rounded-md">
                {filePath}
              </div>
            </div>
          </div>

          {/* Actions */}
          <div className="flex flex-wrap gap-2 mb-6">
            <button
              onClick={handleBack}
              className="px-4 py-2 rounded-full text-sm font-medium bg-[#f5f5f7] text-[#0066cc] hover:bg-[#e5e5ea] transition-all"
            >
              ← Back
            </button>
            {!isEditing ? (
              <button
                onClick={() => setIsEditing(true)}
                className="px-4 py-2 rounded-full text-sm font-medium bg-[#f5f5f7] text-[#0066cc] hover:bg-[#e5e5ea] transition-all"
              >
                ✎ Edit
              </button>
            ) : (
              <>
                <button
                  onClick={handleSave}
                  className="px-4 py-2 rounded-full text-sm font-medium bg-[#f5f5f7] text-[#0066cc] hover:bg-[#e5e5ea] transition-all"
                >
                  💾 Save
                </button>
                <button
                  onClick={() => setIsEditing(false)}
                  className="px-4 py-2 rounded-full text-sm font-medium bg-[#f5f5f7] text-[#0066cc] hover:bg-[#e5e5ea] transition-all"
                >
                  ✕ Cancel
                </button>
              </>
            )}
            <button
              onClick={handleDelete}
              className="px-4 py-2 rounded-full text-sm font-medium bg-[#f5f5f7] text-[#ff3b30] hover:bg-[#fef1f2] transition-all"
            >
              🗑 Delete
            </button>
            <button
              onClick={loadFile}
              className="px-4 py-2 rounded-full text-sm font-medium bg-[#f5f5f7] text-[#0066cc] hover:bg-[#e5e5ea] transition-all"
            >
              ↻ Refresh
            </button>
          </div>

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

          {/* File Content */}
          <div className="rounded-xl overflow-hidden bg-[#f5f5f7] p-6">
            {isLoading ? (
              <div className="text-center text-[#86868b] flex justify-center items-center p-8">
                <svg className="animate-spin -ml-1 mr-3 h-5 w-5 text-[#0066cc]" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24">
                  <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4"></circle>
                  <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"></path>
                </svg>
                Loading...
              </div>
            ) : isEditing ? (
              <textarea
                value={editedContent}
                onChange={(e) => setEditedContent(e.target.value)}
                className="w-full h-96 p-4 font-mono text-sm bg-white rounded-lg border border-[#d2d2d7] focus:outline-none focus:ring-2 focus:ring-[#0066cc] focus:border-transparent"
                spellCheck="false"
              />
            ) : (
              <div className="font-mono text-sm overflow-auto max-h-96 bg-white p-4 rounded-lg border border-[#d2d2d7]">
                {renderContent()}
              </div>
            )}
          </div>
        </div>
      </section>
    </main>
  );
};

export default FileViewer;
