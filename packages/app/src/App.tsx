import React from 'react';
import { Route, Routes } from 'react-router-dom';
import { CanvasView } from './views';

const App: React.FC = () => {
  return (
    <Routes>
      <Route path="/" element={<CanvasView />} />
    </Routes>
  );
};

export default App;
