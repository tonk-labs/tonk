import React, { useState, useRef } from 'react';
import { Upload, FileText, X } from 'lucide-react';

/**
 * Props for the FileLoader component
 */
export interface FileLoaderProps {
  /** Callback function triggered when file is loaded */
  onFileLoaded?: (bytes: Uint8Array, fileName: string, fileSize: number) => void;
  /** Accepted file types (MIME types) */
  acceptedTypes?: string[];
  /** Maximum file size in bytes */
  maxSizeBytes?: number;
  /** Custom button text */
  buttonText?: string;
}

/**
 * A file loader component that allows users to select a file and returns its bytes
 * 
 * @description
 * FileLoader provides a clean interface for file selection with drag-and-drop support.
 * It reads the selected file as bytes and displays file information including size and type.
 * 
 * @example
 * <FileLoader
 *   onFileLoaded={(bytes, name, size) => console.log(`Loaded ${name}: ${size} bytes`)}
 *   acceptedTypes={['image/*', '.pdf']}
 *   maxSizeBytes={5 * 1024 * 1024} // 5MB
 * />
 */
const FileLoader: React.FC<FileLoaderProps> = ({
  onFileLoaded,
  acceptedTypes,
  maxSizeBytes = 10 * 1024 * 1024, // Default 10MB
  buttonText = "Choose File"
}) => {
  const [selectedFile, setSelectedFile] = useState<File | null>(null);
  const [fileBytes, setFileBytes] = useState<Uint8Array | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isDragOver, setIsDragOver] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);

  /**
   * Validates the selected file against size and type constraints
   */
  const validateFile = (file: File): string | null => {
    if (file.size > maxSizeBytes) {
      return `File size (${(file.size / 1024 / 1024).toFixed(2)}MB) exceeds maximum allowed size (${(maxSizeBytes / 1024 / 1024).toFixed(2)}MB)`;
    }

    if (acceptedTypes && acceptedTypes.length > 0) {
      const isAccepted = acceptedTypes.some(type => {
        if (type.startsWith('.')) {
          return file.name.toLowerCase().endsWith(type.toLowerCase());
        }
        if (type.includes('*')) {
          const baseType = type.split('/')[0];
          return file.type.startsWith(baseType);
        }
        return file.type === type;
      });

      if (!isAccepted) {
        return `File type "${file.type}" is not accepted. Accepted types: ${acceptedTypes.join(', ')}`;
      }
    }

    return null;
  };

  /**
   * Processes the selected file and converts it to bytes
   */
  const processFile = async (file: File) => {
    setIsLoading(true);
    setError(null);

    try {
      // Validate file
      const validationError = validateFile(file);
      if (validationError) {
        setError(validationError);
        setIsLoading(false);
        return;
      }

      // Read file as ArrayBuffer and convert to Uint8Array
      const arrayBuffer = await file.arrayBuffer();
      const bytes = new Uint8Array(arrayBuffer);
      
      setSelectedFile(file);
      setFileBytes(bytes);
      
      // Trigger callback if provided
      onFileLoaded?.(bytes, file.name, file.size);
      
    } catch (err) {
      setError(`Failed to read file: ${err instanceof Error ? err.message : 'Unknown error'}`);
    } finally {
      setIsLoading(false);
    }
  };

  /**
   * Handles file input change
   */
  const handleFileChange = (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    if (file) {
      processFile(file);
    }
  };

  /**
   * Handles drag and drop events
   */
  const handleDragOver = (event: React.DragEvent) => {
    event.preventDefault();
    setIsDragOver(true);
  };

  const handleDragLeave = (event: React.DragEvent) => {
    event.preventDefault();
    setIsDragOver(false);
  };

  const handleDrop = (event: React.DragEvent) => {
    event.preventDefault();
    setIsDragOver(false);
    
    const file = event.dataTransfer.files[0];
    if (file) {
      processFile(file);
    }
  };

  /**
   * Clears the selected file and resets state
   */
  const clearFile = () => {
    setSelectedFile(null);
    setFileBytes(null);
    setError(null);
    if (fileInputRef.current) {
      fileInputRef.current.value = '';
    }
  };

  /**
   * Formats file size for display
   */
  const formatFileSize = (bytes: number): string => {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / 1024 / 1024).toFixed(2)} MB`;
  };

  return (
    <div className="w-full max-w-md">
      {/* File input area */}
      <div
        className={`relative border-2 border-dashed rounded-lg p-6 text-center transition-colors ${
          isDragOver
            ? 'border-blue-400 bg-blue-50'
            : error
            ? 'border-red-300 bg-red-50'
            : 'border-gray-300 hover:border-gray-400'
        }`}
        onDragOver={handleDragOver}
        onDragLeave={handleDragLeave}
        onDrop={handleDrop}
      >
        <input
          ref={fileInputRef}
          type="file"
          onChange={handleFileChange}
          accept={acceptedTypes?.join(',')}
          className="absolute inset-0 w-full h-full opacity-0 cursor-pointer"
          disabled={isLoading}
        />
        
        <div className="pointer-events-none">
          {isLoading ? (
            <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600 mx-auto mb-2"></div>
          ) : (
            <Upload className="mx-auto h-8 w-8 text-gray-400 mb-2" />
          )}
          
          <p className="text-sm text-gray-600 mb-2">
            {isLoading ? 'Loading file...' : 'Drag and drop a file here, or click to select'}
          </p>
          
          <button
            type="button"
            disabled={isLoading}
            className="inline-flex items-center px-4 py-2 border border-transparent text-sm font-medium rounded-md text-white bg-blue-600 hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {buttonText}
          </button>
        </div>
      </div>

      {/* Error message */}
      {error && (
        <div className="mt-3 p-3 bg-red-50 border border-red-200 rounded-md">
          <p className="text-sm text-red-600">{error}</p>
        </div>
      )}

      {/* File information */}
      {selectedFile && !error && (
        <div className="mt-4 p-4 bg-gray-50 rounded-lg">
          <div className="flex items-start justify-between">
            <div className="flex items-start space-x-3">
              <FileText className="h-5 w-5 text-gray-400 mt-0.5" />
              <div className="flex-1 min-w-0">
                <p className="text-sm font-medium text-gray-900 truncate">
                  {selectedFile.name}
                </p>
                <p className="text-sm text-gray-500">
                  {formatFileSize(selectedFile.size)} • {selectedFile.type || 'Unknown type'}
                </p>
                {fileBytes && (
                  <p className="text-sm text-gray-500 mt-1">
                    Loaded {fileBytes.length.toLocaleString()} bytes
                  </p>
                )}
              </div>
            </div>
            
            <button
              onClick={clearFile}
              className="ml-2 p-1 text-gray-400 hover:text-gray-600"
              aria-label="Remove file"
            >
              <X className="h-4 w-4" />
            </button>
          </div>

          {/* Byte preview (first 32 bytes) */}
          {fileBytes && (
            <div className="mt-3 pt-3 border-t border-gray-200">
              <p className="text-xs font-medium text-gray-700 mb-2">
                First 32 bytes (hex):
              </p>
              <div className="font-mono text-xs text-gray-600 bg-white p-2 rounded border break-all">
                {Array.from(fileBytes.slice(0, 32))
                  .map(byte => byte.toString(16).padStart(2, '0'))
                  .join(' ')}
                {fileBytes.length > 32 && '...'}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
};

export default FileLoader;
