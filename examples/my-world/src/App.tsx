import React from "react";
import { Route, Routes } from "react-router-dom";
import { MapWorldView } from "./views";

const App: React.FC = () => {
  return (
    <Routes>
      <Route path="/" element={<MapWorldView />} />
    </Routes>
  );
};

export default App;
