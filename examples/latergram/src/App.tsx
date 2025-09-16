import React from 'react';
import { Route, Routes } from 'react-router-dom';
import { CompilerTest } from './views';

const App: React.FC = () => {
  return (
    <Routes>
      <Route path="/" element={<CompilerTest />} />
    </Routes>
  );
};

export default App;
