import { Layers, Database, FileText, House, FolderOpen } from 'lucide-react';
import { Link, Routes, Route, useLocation, useSearchParams, useNavigate } from 'react-router-dom';
import { UnifiedEditor } from '../components/unified-editor';
import AgentChat from './AgentChat';
import { createRef, useCallback } from 'react';

export default function Editor() {
  const chatInputRef = createRef<HTMLInputElement>();
  const location = useLocation();
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const fileParam = searchParams.get('file');

  // Handle file change - update URL with the selected file
  const handleFileChange = useCallback((filePath: string) => {
    const newSearchParams = new URLSearchParams(searchParams);
    newSearchParams.set('file', filePath);
    navigate(`${location.pathname}?${newSearchParams.toString()}`, { replace: true });
  }, [location.pathname, searchParams, navigate]);
  return (
      <div className="h-screen flex flex-col">
        {/* Navigation */}
        <nav className="bg-white border-b border-gray-200 px-6 py-3">
          <div className="flex items-center gap-6">
            <div className="flex items-center gap-1">
              <Link
                to="/"
                className={`flex items-center gap-2 px-3 py-2 rounded-md text-sm transition-colors ${
                  location.pathname === '/'
                }`}
              >
                <House className="w-4 h-4" />
                Home
              </Link>
              <Link
                to="/editor/components"
                className={`flex items-center gap-2 px-3 py-2 rounded-md text-sm transition-colors ${
                  location.pathname === '/editor/components' ||
                  location.pathname === '/editor'
                    ? 'bg-blue-100 text-blue-700'
                    : 'text-gray-600 hover:text-gray-900 hover:bg-gray-100'
                }`}
              >
                <Layers className="w-4 h-4" />
                Components
              </Link>

              <Link
                to="/editor/stores"
                className={`flex items-center gap-2 px-3 py-2 rounded-md text-sm transition-colors ${
                  location.pathname === '/editor/stores'
                    ? 'bg-purple-100 text-purple-700'
                    : 'text-gray-600 hover:text-gray-900 hover:bg-gray-100'
                }`}
              >
                <Database className="w-4 h-4" />
                Stores
              </Link>

              <Link
                to="/editor/pages"
                className={`flex items-center gap-2 px-3 py-2 rounded-md text-sm transition-colors ${
                  location.pathname === '/editor/pages'
                    ? 'bg-green-100 text-green-700'
                    : 'text-gray-600 hover:text-gray-900 hover:bg-gray-100'
                }`}
              >
                <FileText className="w-4 h-4" />
                Pages
              </Link>

              <Link
                to="/editor/files"
                className={`flex items-center gap-2 px-3 py-2 rounded-md text-sm transition-colors ${
                  location.pathname === '/editor/files'
                    ? 'bg-amber-100 text-amber-700'
                    : 'text-gray-600 hover:text-gray-900 hover:bg-gray-100'
                }`}
              >
                <FolderOpen className="w-4 h-4" />
                Files
              </Link>
            </div>
          </div>
        </nav>

        {/* Main Content */}
        <div className="overflow-hidden grid-cols-8 grid h-full">
          <div className="overflow-hidden col-span-6 ">
            <Routes>
              <Route path="/" element={<UnifiedEditor fileFilter="/src/components" initialFile={fileParam} onFileChange={handleFileChange} />} />
              <Route path="components" element={<UnifiedEditor fileFilter="/src/components" initialFile={fileParam} onFileChange={handleFileChange} />} />
              <Route path="stores" element={<UnifiedEditor fileFilter="/src/stores" initialFile={fileParam} onFileChange={handleFileChange} />} />
              <Route path="pages" element={<UnifiedEditor fileFilter="/src/views" initialFile={fileParam} onFileChange={handleFileChange} />} />
              <Route path="files" element={<UnifiedEditor initialFile={fileParam} onFileChange={handleFileChange} />} />
            </Routes>
          </div>
          <div className="overflow-hidden col-span-2 border-l border-gray-200">
            <AgentChat inputRef={chatInputRef} />
          </div>
        </div>
      </div>
  );
}
