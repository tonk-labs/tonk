import type React from 'react';
import { Route, Routes, useParams } from 'react-router-dom';
import { AppInitializer } from './components/AppInitializer';
import { PageLayout } from './components/PageLayout';
import Editor from './components/unified-editor/Editor';
import { ViewRenderer } from './components/ViewRenderer';

// Dynamic view component that maps routes to view files
const DynamicView: React.FC<{ viewName?: string }> = ({ viewName }) => {
  // If no viewName provided, it's a catch-all route, get from URL
  const params = useParams();
  let pathSegments = params['*'] || viewName || 'index';

  // Strip out 'route/' prefix if it exists
  if (pathSegments.startsWith('route/')) {
    pathSegments = pathSegments.slice(6); // Remove 'route/' (6 characters)
  }

  // Construct the view path - map route to file path
  const viewPath = `/src/views/${pathSegments}.tsx`;

  return <ViewRenderer viewPath={viewPath} />;
};

const App: React.FC<{ viewName?: string }> = ({ viewName }) => {
  return (
    <AppInitializer>
      <Routes>
        {/* Editor route - has its own layout */}
        <Route path="/editor/*" element={<Editor />} />

        {/* Page routes with persistent drawer overlay */}
        <Route element={<PageLayout />}>
          <Route path="/" element={<DynamicView viewName="index" />} />
          {/* Catch all route for dynamic views */}
          <Route path="/*" element={<DynamicView />} />
        </Route>
      </Routes>
    </AppInitializer>
  );
};

export default App;
