import React from 'react';
import { Route, Routes, useLocation, useParams } from 'react-router-dom';
import { AppInitializer } from './components/AppInitializer';
import { ViewRenderer } from './components/ViewRenderer';
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

const App: React.FC = () => {
  const location = useLocation();

  return (
    <AppInitializer>
      <Routes>
        <Route path="/" element={<DynamicView viewName="index" />} />
        <Route path="/editor/*" element={<Editor />} />
        {/* Catch all route for dynamic views */}
        <Route path="/*" element={<DynamicView />} />
      </Routes>
    </AppInitializer>
  );
};

export default App;
