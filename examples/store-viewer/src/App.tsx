import React from "react";
import { Route, Routes } from "react-router-dom";
import { StoreViewer } from "./views";

const App: React.FC = () => {
  return (
    <Routes>
      <Route path="/" element={<StoreViewer />} />
    </Routes>
  );
};

export default App;
