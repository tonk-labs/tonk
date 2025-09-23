import React, { useState } from "react";
import { TonkFileSystem } from "../tonk";

/**
 * A simple Hello World view component that demonstrates basic layout and styling
 */
const HelloWorld = () => {
  const [loadedFileInfo, setLoadedFileInfo] = useState<{
    name: string;
    size: number;
  } | null>(null);
  const [files, setFiles] = useState({})
  const [currentDirectory, setCurrentDirectory] = useState<string>('');
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  /**
   * Fetches the bundle from the server and initializes Tonk
   */
  const loadBundleFromServer = async () => {
    setIsLoading(true);
    setError(null);
    
    try {
      const response = await fetch('http://localhost:8081/.manifest.tonk');
      
      if (!response.ok) {
        throw new Error(`Failed to fetch bundle: ${response.status} ${response.statusText}`);
      }
      
      const arrayBuffer = await response.arrayBuffer();
      const bytes = new Uint8Array(arrayBuffer);
      
      setLoadedFileInfo({
        name: 'manifest.tonk',
        size: bytes.length,
      });

      const TonkSingleton = TonkFileSystem.getInstance();
      if (!TonkSingleton.isInitialized) {
        await TonkSingleton.initializeTonk(bytes);
      }

      TonkSingleton.tonk!.connectWebsocket('ws://localhost:8081');
      
      setTimeout(async () => {
        setFiles(await TonkFileSystem.getInstance().vfs!.listDirectory(''));
      }, 50)
      setCurrentDirectory('');
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : 'Unknown error occurred';
      setError(errorMessage);
      console.error('Error loading bundle from server:', err);
    } finally {
      setIsLoading(false);
    }
  };

  const addFile = async () => {
    // const filepath = '/' + Math.random()*100000000;
    // console.log(filepath);
    await TonkFileSystem.getInstance().vfs!.readFile('/app/index.html');
    // console.log(await TonkFileSystem.getInstance().vfs!.readFile(filepath));
    // setFiles(await TonkFileSystem.getInstance().vfs!.listDirectory(''));

  }

  const saveBundle = async () => {
    try {
      const bytes = await TonkFileSystem.getInstance().tonk!.toBytes();
      
      // Create a blob from the bytes - ensure we have a proper ArrayBuffer
      const arrayBuffer = new ArrayBuffer(bytes.length);
      const uint8Array = new Uint8Array(arrayBuffer);
      uint8Array.set(bytes);
      const blob = new Blob([arrayBuffer], { type: 'application/octet-stream' });
      
      // Create a download link
      const url = URL.createObjectURL(blob);
      const link = document.createElement('a');
      link.href = url;
      link.download = 'tonk-saved.bin';
      
      // Trigger the download
      document.body.appendChild(link);
      link.click();
      
      // Clean up
      document.body.removeChild(link);
      URL.revokeObjectURL(url);
      
      console.log('Bundle saved successfully as tonk-saved.bin');
    } catch (error) {
      console.error('Error saving bundle:', error);
    }
  }

  /**
   * Navigate into a folder
   */
  const navigateToFolder = async (folderName: string) => {
    try {
      const newPath = currentDirectory ? `${currentDirectory}/${folderName}` : folderName;
      const updatedFiles = await TonkFileSystem.getInstance().vfs!.listDirectory(newPath);
      setFiles(updatedFiles);
      setCurrentDirectory(newPath);
      console.log('Navigated to:', newPath, updatedFiles);
    } catch (error) {
      console.error('Error navigating to folder:', error);
      setError(`Failed to navigate to folder: ${folderName}`);
    }
  };

  /**
   * Navigate back to parent directory
   */
  const navigateBack = async () => {
    try {
      const pathParts = currentDirectory.split('/').filter(part => part !== '');
      const parentPath = pathParts.slice(0, -1).join('/');
      const updatedFiles = await TonkFileSystem.getInstance().vfs!.listDirectory(parentPath);
      setFiles(updatedFiles);
      setCurrentDirectory(parentPath);
      console.log('Navigated back to:', parentPath, updatedFiles);
    } catch (error) {
      console.error('Error navigating back:', error);
      setError('Failed to navigate back');
    }
  };

  /**
   * Navigate to a specific path from breadcrumb
   */
  const navigateToBreadcrumb = async (targetPath: string) => {
    try {
      const updatedFiles = await TonkFileSystem.getInstance().vfs!.listDirectory(targetPath);
      setFiles(updatedFiles);
      setCurrentDirectory(targetPath);
      console.log('Navigated to breadcrumb path:', targetPath, updatedFiles);
    } catch (error) {
      console.error('Error navigating to breadcrumb path:', error);
      setError(`Failed to navigate to: ${targetPath}`);
    }
  };

  const refreshFiles = async () => {
    try {
      const updatedFiles = await TonkFileSystem.getInstance().vfs!.listDirectory(currentDirectory);
      setFiles(updatedFiles);
      console.log('Files refreshed:', updatedFiles);
    } catch (error) {
      console.error('Error refreshing files:', error);
    }
  }


  return (
    <main className="min-h-screen bg-gray-50 p-4 md:p-6 lg:p-8">
      <section className="max-w-4xl mx-auto">
        <div className="bg-white rounded-lg shadow-sm p-6">
          <div className="mb-6">
            <h1 className="text-3xl font-bold text-gray-900">Hello World</h1>
          </div>
          
          <div className="mb-8">
            <p className="text-gray-600 mb-4">
              Welcome to your new Tonk application! Load the bundle from the server to get started.
            </p>
          </div>

          <div className="mb-8">
            <h2 className="text-xl font-semibold text-gray-900 mb-4">Server Bundle Loader</h2>
            <div className="space-y-4">
              <button
                onClick={loadBundleFromServer}
                disabled={isLoading}
                className="inline-flex items-center px-6 py-3 border border-transparent text-base font-medium rounded-md text-white bg-blue-600 hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 transition-colors"
              >
                {isLoading ? (
                  <>
                    <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-white mr-2"></div>
                    Loading Bundle...
                  </>
                ) : (
                  'Load Bundle from Server'
                )}
              </button>
              
              {error && (
                <div className="p-4 bg-red-50 border border-red-200 rounded-md">
                  <p className="text-sm text-red-600">{error}</p>
                </div>
              )}
              
              {loadedFileInfo && (
                <div className="p-4 bg-green-50 border border-green-200 rounded-md">
                  <div className="flex items-center">
                    <svg className="w-5 h-5 text-green-400 mr-2" fill="currentColor" viewBox="0 0 20 20">
                      <path fillRule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zm3.707-9.293a1 1 0 00-1.414-1.414L9 10.586 7.707 9.293a1 1 0 00-1.414 1.414l2 2a1 1 0 001.414 0l4-4z" clipRule="evenodd" />
                    </svg>
                    <div>
                      <p className="text-sm font-medium text-green-800">
                        Bundle loaded successfully: {loadedFileInfo.name}
                      </p>
                      <p className="text-sm text-green-600">
                        Size: {(loadedFileInfo.size / 1024).toFixed(2)} KB
                      </p>
                    </div>
                  </div>
                </div>
              )}
            </div>
          </div>
           {TonkFileSystem.getInstance().isInitialized && (
            <div className="mb-8">
              <div className="flex items-center justify-between mb-4">
                <div>
                  <h2 className="text-xl font-semibold text-gray-900">File System Contents</h2>
                  {/* Breadcrumb navigation */}
                  <div className="flex items-center space-x-2 mt-2 text-sm text-gray-600">
                    <button
                      onClick={() => navigateToBreadcrumb('')}
                      className="hover:text-blue-600 hover:underline focus:outline-none"
                    >
                      root
                    </button>
                    {currentDirectory && currentDirectory.split('/').filter(part => part !== '').map((part, index, parts) => {
                      const pathToHere = parts.slice(0, index + 1).join('/');
                      return (
                        <React.Fragment key={pathToHere}>
                          <span className="text-gray-400">/</span>
                          <button
                            onClick={() => navigateToBreadcrumb(pathToHere)}
                            className="hover:text-blue-600 hover:underline focus:outline-none"
                          >
                            {part}
                          </button>
                        </React.Fragment>
                      );
                    })}
                  </div>
                </div>
                <div className="flex space-x-3">
                  {currentDirectory && (
                    <button
                      onClick={navigateBack}
                      className="px-4 py-2 bg-gray-500 text-white text-sm font-medium rounded-lg hover:bg-gray-600 focus:outline-none focus:ring-2 focus:ring-gray-500 focus:ring-offset-2 transition-colors"
                    >
                      ← Back
                    </button>
                  )}
                  <button
                    onClick={refreshFiles}
                    className="px-4 py-2 bg-gray-600 text-white text-sm font-medium rounded-lg hover:bg-gray-700 focus:outline-none focus:ring-2 focus:ring-gray-500 focus:ring-offset-2 transition-colors"
                  >
                    Refresh Files
                  </button>
                  <button
                    onClick={addFile}
                    className="px-4 py-2 bg-blue-600 text-white text-sm font-medium rounded-lg hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 transition-colors"
                  >
                    Add Test File
                  </button>
                  <button
                    onClick={saveBundle}
                    className="px-4 py-2 bg-green-600 text-white text-sm font-medium rounded-lg hover:bg-green-700 focus:outline-none focus:ring-2 focus:ring-green-500 focus:ring-offset-2 transition-colors"
                  >
                    Save Bundle
                  </button>
                </div>
              </div>
              <div className="bg-gray-50 rounded-lg p-4 border border-gray-200">
                <div className="space-y-2">
                  {(() => {
                    try {
                      console.log('Files from listDirectory:', files); // Debug log
                      
                      if (Array.isArray(files) && files.length > 0) {
                        return files.map((file: any, index: number) => {
                          const isFolder = file.type === 'directory' || file.type === 'folder';
                          const isClickable = isFolder;
                          
                          return (
                            <div 
                              key={`${file.name}-${index}`} 
                              className={`flex items-center space-x-3 p-2 bg-white rounded border border-gray-100 transition-colors ${
                                isClickable 
                                  ? 'hover:bg-blue-50 hover:border-blue-200 cursor-pointer' 
                                  : 'hover:bg-gray-50'
                              }`}
                              onClick={isClickable ? () => navigateToFolder(file.name) : undefined}
                            >
                              <div className="flex-shrink-0">
                                {file.type === 'document' ? (
                                  <svg className="w-5 h-5 text-blue-500" fill="currentColor" viewBox="0 0 20 20">
                                    <path fillRule="evenodd" d="M4 3a2 2 0 00-2 2v10a2 2 0 002 2h12a2 2 0 002-2V5a2 2 0 00-2-2H4zm12 12H4l4-8 3 6 2-4 3 6z" clipRule="evenodd" />
                                  </svg>
                                ) : isFolder ? (
                                  <svg className="w-5 h-5 text-yellow-500" fill="currentColor" viewBox="0 0 20 20">
                                    <path fillRule="evenodd" d="M4 4a2 2 0 00-2 2v8a2 2 0 002 2h12a2 2 0 002-2V6a2 2 0 00-2-2h-5L9 2H4z" clipRule="evenodd" />
                                  </svg>
                                ) : (
                                  <svg className="w-5 h-5 text-gray-500" fill="currentColor" viewBox="0 0 20 20">
                                    <path fillRule="evenodd" d="M4 3a2 2 0 00-2 2v10a2 2 0 002 2h12a2 2 0 002-2V5a2 2 0 00-2-2H4zm12 12H4l4-8 3 6 2-4 3 6z" clipRule="evenodd" />
                                  </svg>
                                )}
                              </div>
                              <div className="flex-1 min-w-0">
                                <p className={`text-sm font-medium truncate ${
                                  isClickable ? 'text-blue-700' : 'text-gray-900'
                                }`}>
                                  {file.name}
                                  {isClickable && <span className="ml-1 text-gray-400">→</span>}
                                </p>
                                <div className="flex items-center space-x-2">
                                  <p className="text-xs text-gray-500 capitalize">
                                    {file.type}
                                  </p>
                                  {file.size && (
                                    <p className="text-xs text-gray-500">
                                      • {file.size} bytes
                                    </p>
                                  )}
                                </div>
                              </div>
                            </div>
                          );
                        });
                      } else if (typeof files === 'string') {
                        return (
                          <div className="p-3 bg-white rounded border border-gray-100">
                            <pre className="text-sm text-gray-700 whitespace-pre-wrap font-mono">
                              {files}
                            </pre>
                          </div>
                        );
                      } else {
                        return (
                          <div className="p-3 bg-white rounded border border-gray-100 text-center">
                            <p className="text-sm text-gray-500">No files found</p>
                          </div>
                        );
                      }
                    } catch (error) {
                      return (
                        <div className="p-3 bg-red-50 rounded border border-red-200">
                          <p className="text-sm text-red-700">
                            Error reading directory: {error instanceof Error ? error.message : 'Unknown error'}
                          </p>
                        </div>
                      );
                    }
                  })()}
                </div>
              </div>
            </div>
          )}
        </div>
      </section>
    </main>
  );
};

export default HelloWorld;
