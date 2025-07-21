import React from "react";
import { Route, Routes } from "react-router-dom";
import { TransferStores } from "./views";

const App: React.FC = () => {
  return (
    <Routes>
      <Route path="/" element={<TransferStores />} />
    </Routes>
  );
};

export default App;
