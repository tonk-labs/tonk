import React from 'react';
import { Route, Routes } from 'react-router-dom';
import { CompilerTest } from './views';
import HelloWorld from './components/HelloWorld';

const App: React.FC = () => {
  return (
    <Routes>
      <Route path="/" element={<HelloWorld />} />
    </Routes>
  );
};

export default App;
