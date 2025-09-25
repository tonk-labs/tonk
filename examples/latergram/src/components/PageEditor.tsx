import React, { useState, useEffect, useCallback } from 'react';
import { FileText, Save, RefreshCw, AlertCircle, File, Plus, Trash2 } from 'lucide-react';
import { getVFSService } from '../services/vfs-service';
import { ViewRenderer } from './ViewRenderer';

interface PageFile {
  path: string;
  name: string;
  exists: boolean;
}

const DEFAULT_PAGE_TEMPLATE = `export default function HomePage() {
  return (
    <div style={{
      minHeight: '100vh',
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      backgroundColor: '#f3f4f6'
    }}>
      <div style={{ textAlign: 'center', padding: '2rem' }}>
        <h1 style={{
          fontSize: '3rem',
          fontWeight: 'bold',
          color: '#1f2937',
          marginBottom: '1rem'
        }}>
          Welcome to Latergram
        </h1>
        <p style={{
          fontSize: '1.25rem',
          color: '#6b7280',
          marginBottom: '2rem'
        }}>
          Build amazing experiences with components
        </p>
        <button
          onClick={() => window.location.href = '/editor/components'}
          style={{
            padding: '0.75rem 2rem',
            backgroundColor: '#3b82f6',
            color: 'white',
            border: 'none',
            borderRadius: '0.5rem',
            fontSize: '1.125rem',
            cursor: 'pointer'
          }}
        >
          Open Component Editor
        </button>
      </div>
    </div>
  );
}`;

export const PageEditor: React.FC = () => {
  const [selectedPage, setSelectedPage] = useState<string>('/src/views/index.tsx');
  const [pageContent, setPageContent] = useState<string>('');
  const [originalContent, setOriginalContent] = useState<string>('');
  const [pages, setPages] = useState<PageFile[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<'edit' | 'preview'>('preview');
  const [showNewPageDialog, setShowNewPageDialog] = useState(false);
  const [newPageName, setNewPageName] = useState('');

  const vfs = getVFSService();

  // Discover available pages
  const discoverPages = useCallback(async () => {
    if (!vfs.isInitialized()) return;

    const discoveredPages: PageFile[] = [];

    // Check if index.tsx exists (it's special - always show it)
    try {
      const indexExists = await vfs.exists('/src/views/index.tsx');
      discoveredPages.push({
        path: '/src/views/index.tsx',
        name: 'Home Page (index)',
        exists: indexExists
      });
    } catch (error) {
      console.error('Failed to check index.tsx:', error);
      discoveredPages.push({
        path: '/src/views/index.tsx',
        name: 'Home Page (index)',
        exists: false
      });
    }

    // Check for any other .tsx files in /src/views
    try {
      const files = await vfs.listDirectory('/src/views');
      for (const file of files as any[]) {
        const fileName = typeof file === 'string' ? file : file.name || file.path;
        if (fileName && fileName.endsWith('.tsx') && fileName !== 'index.tsx') {
          const fullPath = `/src/views/${fileName}`;
          const name = fileName.replace('.tsx', '').replace(/-/g, ' ')
            .split(' ').map(word => word.charAt(0).toUpperCase() + word.slice(1)).join(' ');
          discoveredPages.push({
            path: fullPath,
            name: name,
            exists: true
          });
        }
      }
    } catch (error) {
      console.warn('Could not list /src/views directory:', error);
    }

    setPages(discoveredPages);
  }, [vfs]);

  // Load page content
  const loadPage = useCallback(async (pagePath: string) => {
    if (!vfs.isInitialized()) return;

    setIsLoading(true);
    setError(null);

    try {
      const exists = await vfs.exists(pagePath);
      if (!exists) {
        setPageContent(DEFAULT_PAGE_TEMPLATE);
        setOriginalContent('');
      } else {
        const content = await vfs.readFile(pagePath);
        setPageContent(content);
        setOriginalContent(content);
      }
    } catch (error) {
      console.error('Failed to load page:', error);
      setError(`Failed to load page: ${error instanceof Error ? error.message : 'Unknown error'}`);
      setPageContent('');
      setOriginalContent('');
    } finally {
      setIsLoading(false);
    }
  }, [vfs]);

  // Save page content
  const savePage = useCallback(async () => {
    if (!vfs.isInitialized() || !selectedPage) return;

    setIsSaving(true);
    setError(null);

    try {
      const exists = await vfs.exists(selectedPage);
      await vfs.writeFile(selectedPage, pageContent, !exists);
      setOriginalContent(pageContent);

      // Refresh pages list
      await discoverPages();
    } catch (error) {
      console.error('Failed to save page:', error);
      setError(`Failed to save page: ${error instanceof Error ? error.message : 'Unknown error'}`);
    } finally {
      setIsSaving(false);
    }
  }, [vfs, selectedPage, pageContent, discoverPages]);

  // Create new page
  const createNewPage = useCallback(async () => {
    if (!vfs.isInitialized() || !newPageName.trim()) return;

    const fileName = newPageName.toLowerCase().replace(/\s+/g, '-') + '.tsx';
    const filePath = `/src/views/${fileName}`;

    try {
      // Check if file already exists
      const exists = await vfs.exists(filePath);
      if (exists) {
        setError(`Page ${fileName} already exists`);
        return;
      }

      // Create the new page with template
      const customTemplate = DEFAULT_PAGE_TEMPLATE.replace('HomePage', newPageName.replace(/\s+/g, ''));
      await vfs.writeFile(filePath, customTemplate, true);

      // Refresh pages list and select the new page
      await discoverPages();
      setSelectedPage(filePath);
      await loadPage(filePath);

      // Close dialog
      setShowNewPageDialog(false);
      setNewPageName('');
    } catch (error) {
      console.error('Failed to create new page:', error);
      setError(`Failed to create page: ${error instanceof Error ? error.message : 'Unknown error'}`);
    }
  }, [vfs, newPageName, discoverPages, loadPage]);

  // Delete page
  const deletePage = useCallback(async (pagePath: string) => {
    if (!vfs.isInitialized()) return;

    if (!confirm(`Are you sure you want to delete this page?`)) return;

    try {
      await vfs.deleteFile(pagePath);

      // If we deleted the selected page, select another one
      if (pagePath === selectedPage) {
        setSelectedPage('/src/views/index.tsx');
      }

      // Refresh pages list
      await discoverPages();
    } catch (error) {
      console.error('Failed to delete page:', error);
      setError(`Failed to delete page: ${error instanceof Error ? error.message : 'Unknown error'}`);
    }
  }, [vfs, selectedPage, discoverPages]);

  // Initialize
  useEffect(() => {
    if (vfs.isInitialized()) {
      discoverPages();
    }
  }, [discoverPages, vfs]);

  // Load selected page
  useEffect(() => {
    if (vfs.isInitialized() && selectedPage) {
      loadPage(selectedPage);
    }
  }, [selectedPage, loadPage, vfs]);

  const hasChanges = pageContent !== originalContent;

  return (
    <div className="flex h-full bg-gray-100 overflow-hidden">
      {/* Sidebar - Pages List */}
      <div className="w-64 bg-white border-r border-gray-200 flex flex-col">
        <div className="p-4 border-b border-gray-200">
          <div className="flex items-center justify-between mb-3">
            <h3 className="font-semibold text-gray-800">Pages</h3>
            <button
              onClick={() => setShowNewPageDialog(true)}
              className="p-1 hover:bg-gray-100 rounded transition-colors"
              title="Create new page"
            >
              <Plus className="w-4 h-4 text-gray-600" />
            </button>
          </div>
        </div>

        <div className="flex-1 overflow-y-auto p-2">
          {pages.length === 0 ? (
            <div className="text-center py-8 px-4">
              <FileText className="w-12 h-12 text-gray-300 mx-auto mb-3" />
              <p className="text-sm text-gray-500 mb-4">No pages yet</p>
              <button
                onClick={() => setShowNewPageDialog(true)}
                className="text-xs text-blue-500 hover:text-blue-600"
              >
                Create your first page
              </button>
            </div>
          ) : (
            pages.map(page => (
              <div
                key={page.path}
                onClick={() => setSelectedPage(page.path)}
                className={`
                  p-2 mb-1 rounded cursor-pointer transition-all group
                  ${selectedPage === page.path
                    ? 'bg-blue-50 border border-blue-200'
                    : 'hover:bg-gray-50 border border-transparent'
                  }
                `}
              >
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2 flex-1 min-w-0">
                    <File className={`w-4 h-4 ${page.exists ? 'text-blue-500' : 'text-gray-400'}`} />
                    <div className="flex-1 min-w-0">
                      <div className="text-sm font-medium text-gray-800 truncate">
                        {page.name}
                      </div>
                      <div className="text-xs text-gray-500 truncate">
                        {page.path.replace('/src/views/', '')}
                      </div>
                    </div>
                  </div>
                  {page.exists && page.path !== '/src/views/index.tsx' && (
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        deletePage(page.path);
                      }}
                      className="opacity-0 group-hover:opacity-100 p-1 hover:bg-red-100 rounded transition-all"
                      title="Delete page"
                    >
                      <Trash2 className="w-3 h-3 text-red-500" />
                    </button>
                  )}
                </div>
                {!page.exists && (
                  <div className="text-xs text-amber-600 mt-1 ml-6">
                    Click to create
                  </div>
                )}
              </div>
            ))
          )}
      </div>

      {/* Main Content Area */}
      <div className="flex-1 flex flex-col overflow-hidden">
        {/* Header */}
        <div className="bg-white border-b border-gray-200 px-6 py-3">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-4">
              <FileText className="w-5 h-5 text-blue-500" />
              <h2 className="text-lg font-semibold text-gray-800">
                Page Editor
              </h2>
              <span className="text-sm text-gray-500">
                {selectedPage}
              </span>
            </div>

            <div className="flex items-center gap-2">
              {hasChanges && (
                <span className="text-xs text-amber-600 flex items-center gap-1">
                  <AlertCircle className="w-3 h-3" />
                  Unsaved changes
                </span>
              )}
              <button
                onClick={savePage}
                disabled={!hasChanges || isSaving}
                className={`
                  flex items-center gap-2 px-4 py-2 rounded-lg transition-colors
                  ${hasChanges
                    ? 'bg-blue-500 text-white hover:bg-blue-600'
                    : 'bg-gray-100 text-gray-400 cursor-not-allowed'
                  }
                `}
              >
                {isSaving ? (
                  <RefreshCw className="w-4 h-4 animate-spin" />
                ) : (
                  <Save className="w-4 h-4" />
                )}
                Save
              </button>
            </div>
          </div>
        </div>

        {/* Tab Bar */}
        <div className="bg-white border-b border-gray-200 px-6">
          <div className="flex gap-1">
            <button
              onClick={() => setActiveTab('preview')}
              className={`px-4 py-2 font-medium text-sm border-b-2 transition-colors ${
                activeTab === 'preview'
                  ? 'border-blue-500 text-blue-600'
                  : 'border-transparent text-gray-500 hover:text-gray-700'
              }`}
            >
              Preview
            </button>
            <button
              onClick={() => setActiveTab('edit')}
              className={`px-4 py-2 font-medium text-sm border-b-2 transition-colors ${
                activeTab === 'edit'
                  ? 'border-blue-500 text-blue-600'
                  : 'border-transparent text-gray-500 hover:text-gray-700'
              }`}
            >
              Edit
            </button>
          </div>
        </div>

        {/* Error Display */}
        {error && (
          <div className="bg-red-50 border-b border-red-200 px-6 py-3">
            <div className="flex items-center gap-2 text-sm text-red-700">
              <AlertCircle className="w-4 h-4" />
              {error}
            </div>
          </div>
        )}

        {/* Content Area */}
        <div className="flex-1 overflow-hidden">
          {isLoading ? (
            <div className="flex items-center justify-center h-full bg-gray-50">
              <div className="text-center">
                <RefreshCw className="w-8 h-8 animate-spin text-blue-500 mx-auto mb-4" />
                <p className="text-gray-600">Loading page...</p>
              </div>
            </div>
          ) : activeTab === 'edit' ? (
            <div className="h-full bg-gray-900">
              <textarea
                value={pageContent}
                onChange={(e) => setPageContent(e.target.value)}
                className="w-full h-full p-6 font-mono text-sm text-gray-100 bg-gray-900 resize-none focus:outline-none"
                spellCheck={false}
                placeholder="// Start writing your page component..."
              />
            </div>
          ) : (
            <div className="h-full overflow-auto bg-white">
              <ViewRenderer
                viewPath={selectedPage}
                className="min-h-full"
              />
            </div>
          )}
        </div>
      </div>

      {/* New Page Dialog */}
      {showNewPageDialog && (
        <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
          <div className="bg-white rounded-lg p-6 w-full max-w-md mx-4">
            <h3 className="text-lg font-semibold mb-4">Create New Page</h3>

            <div className="mb-4">
              <label className="block text-sm font-medium text-gray-700 mb-2">
                Page Name
              </label>
              <input
                type="text"
                value={newPageName}
                onChange={(e) => setNewPageName(e.target.value)}
                placeholder="e.g., About Us"
                className="w-full px-3 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
                autoFocus
              />
              <p className="text-xs text-gray-500 mt-1">
                Will be saved as: /src/views/{newPageName.toLowerCase().replace(/\s+/g, '-')}.tsx
              </p>
            </div>

            <div className="flex gap-3">
              <button
                onClick={() => {
                  setShowNewPageDialog(false);
                  setNewPageName('');
                }}
                className="flex-1 px-4 py-2 border border-gray-300 text-gray-700 rounded-lg hover:bg-gray-50 transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={createNewPage}
                disabled={!newPageName.trim()}
                className="flex-1 px-4 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
              >
                Create
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};