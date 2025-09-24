import React from 'react';
import { Route, Routes, Link, useLocation } from 'react-router-dom';
import { CompilerTest } from './views';
import { StoreManager } from './components/StoreManager';
import { Layers, Database } from 'lucide-react';

const App: React.FC = () => {
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
                  ? 'bg-blue-100 text-blue-700'
                  : 'text-gray-600 hover:text-gray-900 hover:bg-gray-100'
              }`}
            >
              <Layers className="w-4 h-4" />
              Components
            </Link>

            <Link
              to="/stores"
              className={`flex items-center gap-2 px-3 py-2 rounded-md text-sm transition-colors ${
                location.pathname === '/stores'
                  ? 'bg-purple-100 text-purple-700'
                  : 'text-gray-600 hover:text-gray-900 hover:bg-gray-100'
              }`}
            >
              <Database className="w-4 h-4" />
              Stores
            </Link>
          </div>
        </div>
      </nav>

      {/* Main Content */}
      <div className="flex-1 overflow-hidden">
        <Routes>
          <Route path="/" element={<CompilerTest />} />
          <Route path="/stores" element={<StoreManager />} />
        </Routes>
      </div>
    </div>
  );
};

export default App;
