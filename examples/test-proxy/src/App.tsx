import React from "react";
import { Route, Routes } from "react-router-dom";
import { HelloWorld } from "./views";
import { ApiTester } from "./components/ApiTester";

const App: React.FC = () => {
  return (
    <Routes>
      <Route path="/" element={<HelloWorld />} />
      <Route path="/api-test" element={<ApiTester />} />
    </Routes>
  );
};

export default App;
