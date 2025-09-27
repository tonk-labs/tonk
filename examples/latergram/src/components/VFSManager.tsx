import React, { useState } from 'react';
import { VFSFileBrowser } from './VFSFileBrowser';
import { VFSFilePreview } from './VFSFilePreview';
import { EditorSidebar } from './shared/EditorSidebar';
import { FolderOpen } from 'lucide-react';

export const VFSManager: React.FC = () => {
  const [selectedFile, setSelectedFile] = useState<string | null>(null);

  const handleFileSelect = (path: string) => {
    setSelectedFile(path);
  };

  const handleFileDelete = (path: string) => {
    if (selectedFile === path) {
      setSelectedFile(null);
    }
  };

  return (
    <div className="flex h-full bg-gray-50">
      <EditorSidebar title="File Browser">
        <VFSFileBrowser
          onFileSelect={handleFileSelect}
          onFileDelete={handleFileDelete}
          className="flex-1 overflow-hidden"
        />
      </EditorSidebar>

      <div className="flex-1 bg-white">
        {selectedFile ? (
          <VFSFilePreview
            filePath={selectedFile}
            onClose={() => setSelectedFile(null)}
          />
        ) : (
          <div className="flex items-center justify-center h-full text-gray-400">
            <div className="text-center">
              <FolderOpen className="w-16 h-16 mx-auto mb-4" />
              <p className="text-lg">Select a file from the browser</p>
              <p className="text-sm mt-2">You can view and delete files in your VFS</p>
            </div>
          </div>
        )}
      </div>
    </div>
  );
};