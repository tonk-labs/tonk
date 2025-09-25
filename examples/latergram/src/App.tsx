import React from 'react';
import { Route, Routes, useLocation } from 'react-router-dom';
import { AppInitializer } from './components/AppInitializer';
import { ViewRenderer } from './components/ViewRenderer';
import Editor from './views/Editor';

const App: React.FC = () => {
  const location = useLocation();

  return (
    <AppInitializer>
      <Routes>
        <Route path="/" element={
          <ViewRenderer
            viewPath="/src/views/index.tsx"
            showEditor={true}
          />
        } />
        <Route path="/editor/*" element={<Editor />} />
      </Routes>
    </AppInitializer>
  );
};

export default App;
