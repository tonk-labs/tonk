import React from "react";
import { Route, Routes } from "react-router-dom";
import { HelloWorld, Todos } from "./views";

const App: React.FC = () => {
  return (
    <Routes>
      <Route path="/" element={<Todos />} />
      <Route path="/hello" element={<HelloWorld />} />
    </Routes>
  );
};

export default App;
