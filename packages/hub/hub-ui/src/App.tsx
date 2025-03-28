import React from "react";
import { HashRouter, Routes, Route } from "react-router-dom";
import Main from "./views/Main";

const App: React.FC = () => {
  return (
    <HashRouter>
      <Routes>
        <Route path="/" element={<Main />} />
      </Routes>
    </HashRouter>
  );
};

export default App;
