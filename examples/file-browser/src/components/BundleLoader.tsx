import React, { useState, useRef } from 'react';
import { Bundle, Manifest } from '@tonk/core';

export interface BundleLoaderProps {
  onBundleLoad: (bytes: Uint8Array, manifest: Manifest) => void;
  isLoading?: boolean;
}

const BundleLoader: React.FC<BundleLoaderProps> = ({
  onBundleLoad,
  isLoading = false,
}) => {
  const [error, setError] = useState<string | null>(null);
  const [isDragging, setIsDragging] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const handleFile = async (file: File) => {
    setError(null);

    if (!file.name.endsWith('.tonk')) {
      setError('Please select a .tonk bundle file');
      return;
    }

    try {
      const buffer = await file.arrayBuffer();
      const bytes = new Uint8Array(buffer);

      // Validate bundle by parsing it
      const bundle = await Bundle.fromBytes(bytes);
      const manifest = await bundle.getManifest();
      bundle.free();

      if (!manifest.entrypoints || manifest.entrypoints.length === 0) {
        setError('Bundle has no entrypoints');
        return;
      }

      onBundleLoad(bytes, manifest);
    } catch (err) {
      setError(
        err instanceof Error ? err.message : 'Failed to load bundle'
      );
    }
  };

  const handleFileInput = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) {
      handleFile(file);
    }
  };

  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(false);
    const file = e.dataTransfer.files[0];
    if (file) {
      handleFile(file);
    }
  };

  const handleDragOver = (e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(true);
  };

  const handleDragLeave = (e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(false);
  };

  return (
    <div className="min-h-screen flex items-center justify-center bg-[#f5f5f7]">
      <div className="text-center max-w-md w-full mx-4">
        <div className="bg-white rounded-2xl shadow-md p-8">
          <h1 className="text-2xl font-medium mb-2 text-[#1d1d1f]">
            Tonk File Browser
          </h1>
          <p className="text-[#86868b] mb-6">
            Load a .tonk bundle to browse its virtual file system
          </p>

          <div
            onDrop={handleDrop}
            onDragOver={handleDragOver}
            onDragLeave={handleDragLeave}
            onClick={() => fileInputRef.current?.click()}
            className={`
              border-2 border-dashed rounded-xl p-8 cursor-pointer transition-all
              ${isDragging
                ? 'border-[#0066cc] bg-[#f0f7ff]'
                : 'border-[#d2d2d7] hover:border-[#0066cc] hover:bg-[#fafafa]'
              }
              ${isLoading ? 'opacity-50 pointer-events-none' : ''}
            `}
          >
            <input
              ref={fileInputRef}
              type="file"
              accept=".tonk"
              onChange={handleFileInput}
              className="hidden"
              disabled={isLoading}
            />

            {isLoading ? (
              <div className="flex flex-col items-center">
                <svg
                  className="animate-spin h-10 w-10 text-[#0066cc] mb-3"
                  xmlns="http://www.w3.org/2000/svg"
                  fill="none"
                  viewBox="0 0 24 24"
                >
                  <circle
                    className="opacity-25"
                    cx="12"
                    cy="12"
                    r="10"
                    stroke="currentColor"
                    strokeWidth="4"
                  ></circle>
                  <path
                    className="opacity-75"
                    fill="currentColor"
                    d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
                  ></path>
                </svg>
                <span className="text-[#1d1d1f]">Loading bundle...</span>
              </div>
            ) : (
              <div className="flex flex-col items-center">
                <svg
                  className="w-12 h-12 text-[#0066cc] mb-3"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={1.5}
                    d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12"
                  />
                </svg>
                <span className="text-[#1d1d1f] font-medium mb-1">
                  Drop .tonk file here
                </span>
                <span className="text-[#86868b] text-sm">
                  or click to browse
                </span>
              </div>
            )}
          </div>

          {error && (
            <div className="mt-4 p-3 bg-[#fef1f2] border border-[#ffccd0] rounded-lg">
              <span className="text-[#ff3b30] text-sm">{error}</span>
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

export default BundleLoader;
