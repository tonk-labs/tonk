import { Layers, Database, FileText, House, FolderOpen } from 'lucide-react';
import { Link, Routes, Route, useLocation } from 'react-router-dom';
import { StoreManager } from '../components/StoreManager';
import AgentChat from './AgentChat';
import { ComponentManager } from '../components/ComponentManager';
import { PageEditor } from '../components/PageEditor';
import { VFSManager } from '../components/VFSManager';
import { createRef } from 'react';

export default function Editor() {
  const chatInputRef = createRef<HTMLInputElement>();
  const location = useLocation();
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
              <Route path="/" element={<ComponentManager />} />
              <Route path="components" element={<ComponentManager />} />
              <Route path="stores" element={<StoreManager />} />
              <Route path="pages" element={<PageEditor />} />
              <Route path="files" element={<VFSManager />} />
            </Routes>
          </div>
          <div className="overflow-hidden col-span-2 border-l border-gray-200">
            <AgentChat inputRef={chatInputRef} />
          </div>
        </div>
      </div>
  );
}
