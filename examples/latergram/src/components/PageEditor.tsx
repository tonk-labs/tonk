import React, { useState, useEffect, useCallback } from 'react';
import {
  FileText,
  AlertCircle,
  File,
  RefreshCw,
  Clock,
  Eye,
} from 'lucide-react';
import {
  useLocation,
  useSearchParams,
  useNavigate,
  Link,
} from 'react-router-dom';
import { getVFSService } from '../services/vfs-service';
import { ViewRenderer } from './ViewRenderer';
import {
  EditorSidebar,
  SearchInput,
  SidebarItem,
  EmptyState,
  useAutoSave,
} from './shared';

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
  const [searchParams] = useSearchParams();
  const navigate = useNavigate();
  const fileFromUrl = searchParams.get('file');

  const [selectedPage, setSelectedPage] = useState<string>(
    fileFromUrl || '/src/views/index.tsx'
  );
  const [pageContent, setPageContent] = useState<string>('');
  const [pages, setPages] = useState<PageFile[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<'edit' | 'preview'>('preview');
  const [showNewPageDialog, setShowNewPageDialog] = useState(false);
  const [newPageName, setNewPageName] = useState('');
  const [searchTerm, setSearchTerm] = useState('');

  const vfs = getVFSService();

  // Function to handle page selection and update URL
  const handlePageSelect = useCallback(
    (pagePath: string) => {
      setSelectedPage(pagePath);
      // Update the URL with the new file parameter
      navigate(`/editor/pages?file=${encodeURIComponent(pagePath)}`);
    },
    [navigate]
  );

  // Handle URL parameter changes
  useEffect(() => {
    if (fileFromUrl && fileFromUrl !== selectedPage) {
      setSelectedPage(fileFromUrl);
      // Also ensure the file is in our pages list
      if (!pages.some(p => p.path === fileFromUrl)) {
        const pageName = fileFromUrl
          .replace('/src/views/', '')
          .replace('.tsx', '');
        setPages(prev => [
          ...prev,
          {
            path: fileFromUrl,
            name:
              pageName === 'index'
                ? 'Home Page (index)'
                : pageName.replace(/-/g, ' '),
            exists: true, // Will be checked when loading
          },
        ]);
      }
    }
  }, [fileFromUrl, selectedPage, pages]);

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
        exists: indexExists,
      });
    } catch (error) {
      console.error('Failed to check index.tsx:', error);
      discoveredPages.push({
        path: '/src/views/index.tsx',
        name: 'Home Page (index)',
        exists: false,
      });
    }

    // Check for any other .tsx files in /src/views
    try {
      const files = await vfs.listDirectory('/src/views');
      for (const file of files as any[]) {
        const fileName =
          typeof file === 'string' ? file : file.name || file.path;
        if (fileName && fileName.endsWith('.tsx') && fileName !== 'index.tsx') {
          const fullPath = `/src/views/${fileName}`;
          const name = fileName
            .replace('.tsx', '')
            .replace(/-/g, ' ')
            .split(' ')
            .map(word => word.charAt(0).toUpperCase() + word.slice(1))
            .join(' ');
          discoveredPages.push({
            path: fullPath,
            name: name,
            exists: true,
          });
        }
      }
    } catch (error) {
      console.warn('Could not list /src/views directory:', error);
    }

    setPages(discoveredPages);
  }, [vfs]);

  // Load page content
  const loadPage = useCallback(
    async (pagePath: string) => {
      if (!vfs.isInitialized()) return;

      setIsLoading(true);
      setError(null);

      try {
        const exists = await vfs.exists(pagePath);
        if (!exists) {
          setPageContent(DEFAULT_PAGE_TEMPLATE);
        } else {
          const content = await vfs.readFile(pagePath);
          setPageContent(content);
        }
      } catch (error) {
        console.error('Failed to load page:', error);
        setError(
          `Failed to load page: ${error instanceof Error ? error.message : 'Unknown error'}`
        );
        setPageContent('');
        setOriginalContent('');
      } finally {
        setIsLoading(false);
      }
    },
    [vfs]
  );

  // Save page content function for auto-save
  const savePage = useCallback(
    async (content: string) => {
      if (!vfs.isInitialized() || !selectedPage) return;

      try {
        const exists = await vfs.exists(selectedPage);
        await vfs.writeFile(selectedPage, content, !exists);
        // Refresh pages list if new file
        if (!exists) {
          await discoverPages();
        }
        return true;
      } catch (error) {
        console.error('Failed to save page:', error);
        throw error;
      }
    },
    [vfs, selectedPage, discoverPages]
  );

  // Use auto-save hook
  const { isSaving, lastSaved, hasChanges } = useAutoSave({
    content: pageContent,
    onSave: savePage,
    debounceMs: 1000,
    enabled: true,
  });

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
      const customTemplate = DEFAULT_PAGE_TEMPLATE.replace(
        'HomePage',
        newPageName.replace(/\s+/g, '')
      );
      await vfs.writeFile(filePath, customTemplate, true);

      // Refresh pages list and select the new page
      await discoverPages();
      handlePageSelect(filePath);
      await loadPage(filePath);

      // Close dialog
      setShowNewPageDialog(false);
      setNewPageName('');
    } catch (error) {
      console.error('Failed to create new page:', error);
      setError(
        `Failed to create page: ${error instanceof Error ? error.message : 'Unknown error'}`
      );
    }
  }, [vfs, newPageName, discoverPages, loadPage]);

  // Delete page
  const deletePage = useCallback(
    async (pagePath: string) => {
      if (!vfs.isInitialized()) return;

      if (!confirm(`Are you sure you want to delete this page?`)) return;

      try {
        await vfs.deleteFile(pagePath);

        // If we deleted the selected page, select another one
        if (pagePath === selectedPage) {
          handlePageSelect('/src/views/index.tsx');
        }

        // Refresh pages list
        await discoverPages();
      } catch (error) {
        console.error('Failed to delete page:', error);
        setError(
          `Failed to delete page: ${error instanceof Error ? error.message : 'Unknown error'}`
        );
      }
    },
    [vfs, selectedPage, discoverPages]
  );

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

  // Filter pages based on search
  const filteredPages = pages.filter(
    page =>
      page.name.toLowerCase().includes(searchTerm.toLowerCase()) ||
      page.path.toLowerCase().includes(searchTerm.toLowerCase())
  );

  return (
    <>
      <div className="flex h-full bg-gray-100 overflow-hidden">
        {/* Sidebar - Pages List */}
        <EditorSidebar
          title="Pages"
          onCreateClick={() => setShowNewPageDialog(true)}
        >
          <div className="p-4 border-b border-gray-200">
            <SearchInput
              value={searchTerm}
              onChange={setSearchTerm}
              placeholder="Search pages..."
              focusColor="green"
            />
          </div>

          <div className="flex-1 overflow-y-auto">
            {filteredPages.length === 0 ? (
              <EmptyState
                icon={<FileText className="w-12 h-12 text-gray-300" />}
                title={
                  pages.length === 0
                    ? 'No pages yet'
                    : 'No pages match your search'
                }
                subtitle={
                  pages.length === 0
                    ? 'Create your first page to get started'
                    : undefined
                }
                actionText={
                  pages.length === 0 ? 'Create your first page' : undefined
                }
                onAction={
                  pages.length === 0
                    ? () => setShowNewPageDialog(true)
                    : undefined
                }
              />
            ) : (
              <div className="p-2">
                {filteredPages.map(page => (
                  <SidebarItem
                    key={page.path}
                    selected={selectedPage === page.path}
                    onClick={() => handlePageSelect(page.path)}
                    onDelete={
                      page.path !== '/src/views/index.tsx'
                        ? () => deletePage(page.path)
                        : undefined
                    }
                    icon={
                      <File
                        className={`w-4 h-4 ${page.exists ? 'text-green-500' : 'text-gray-400'}`}
                      />
                    }
                    title={page.name}
                    subtitle={page.path.replace('/src/views/', '')}
                    color="green"
                    canDelete={
                      page.exists && page.path !== '/src/views/index.tsx'
                    }
                  >
                    {!page.exists && (
                      <div className="text-xs text-amber-600 mt-1 ml-6">
                        Click to create
                      </div>
                    )}
                  </SidebarItem>
                ))}
              </div>
            )}
          </div>
        </EditorSidebar>
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
                <span className="text-sm text-gray-500">{selectedPage}</span>
              </div>

              <div className="flex items-center gap-2">
                {isSaving && (
                  <span className="text-xs text-blue-600 flex items-center gap-1">
                    <RefreshCw className="w-3 h-3 animate-spin" />
                    Saving...
                  </span>
                )}
                {!isSaving && lastSaved && (
                  <span className="text-xs text-gray-500 flex items-center gap-1">
                    <Clock className="w-3 h-3" />
                    Auto-saved
                  </span>
                )}
                {hasChanges && !isSaving && (
                  <span className="text-xs text-amber-600">• Unsaved</span>
                )}
              </div>
              <div className="flex grow"/>
                <Link
                  to={
                    '/' +
                    selectedPage.replace('/src/views/', '').replace('.tsx', '')
                  }
                >
                  <div className="flex flex-row items-center items-justify gap-2">
                    <Eye className="w-4 h-4 text-blue-500" />
                    <h2 className="underline text-lg font-semibold text-gray-800">
                      View page
                    </h2>
                  </div>
                </Link>
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
          <div className="flex-1 overflow-hidden relative">
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
                  onChange={e => setPageContent(e.target.value)}
                  className="w-full h-full p-6 font-mono text-sm text-gray-100 bg-gray-900 resize-none focus:outline-none"
                  spellCheck={false}
                  placeholder="// Start writing your page component..."
                />
              </div>
            ) : (
              <div className="h-full overflow-auto bg-white relative">
                {/* Container with isolation to prevent fixed elements from escaping */}
                <div
                  style={{
                    isolation: 'isolate',
                    transform: 'translateZ(0)',
                    position: 'relative',
                    zIndex: 0,
                  }}
                >
                  <ViewRenderer
                    viewPath={selectedPage}
                    className="min-h-full"
                    showEditor={false}
                  />
                </div>
              </div>
            )}
          </div>
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
                onChange={e => setNewPageName(e.target.value)}
                placeholder="e.g., About Us"
                className="w-full px-3 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
                autoFocus
              />
              <p className="text-xs text-gray-500 mt-1">
                Will be saved as: /src/views/
                {newPageName.toLowerCase().replace(/\s+/g, '-')}.tsx
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
    </>
  );
};
