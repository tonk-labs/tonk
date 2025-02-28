import React from "react";
import { Route, Routes } from "react-router-dom";
import { HelloWorld } from "./views";

const App: React.FC = () => {
  return (
    <Routes>
      <Route path="/" element={<HelloWorld />} />
    </Routes>
  );
};

export default App;
