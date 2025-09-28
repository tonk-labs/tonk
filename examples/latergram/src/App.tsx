import React from 'react';
import { Route, Routes, useLocation, useParams } from 'react-router-dom';
import { AppInitializer } from './components/AppInitializer';
import { ViewRenderer } from './components/ViewRenderer';
import { PageLayout } from './components/PageLayout';
import Editor from './views/Editor';

// Dynamic view component that maps routes to view files
const DynamicView: React.FC<{ viewName?: string }> = ({ viewName }) => {
  // If no viewName provided, it's a catch-all route, get from URL
  const params = useParams();
  const pathSegments = params['*'] || viewName || 'index';

  // Construct the view path - map route to file path
  const viewPath = `/src/views/${pathSegments}.tsx`;

  return (
    <ViewRenderer
      viewPath={viewPath}
      showEditor={true}
    />
  );
};

const App: React.FC<{ viewName?: string }> = ({ viewName }) => {
  const params = useParams();
  const pathSegments = params['*'] || viewName || 'index';
  const viewPath = `/src/views/${pathSegments}.tsx`;

  return (
    <AppInitializer>
      <Routes>
        {/* Editor route - has its own layout */}
        <Route path="/editor/*" element={<Editor />} />

        {/* Page routes with persistent drawer overlay */}
        <Route element={<PageLayout viewPath={viewPath} />}>
          <Route path="/" element={<DynamicView viewName="index" />} />
          {/* Catch all route for dynamic views */}
          <Route path="/*" element={<DynamicView />} />
        </Route>
      </Routes>
    </AppInitializer>
  );
};

export default App;
