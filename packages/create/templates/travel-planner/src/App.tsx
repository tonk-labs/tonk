import React from "react";
import { Route, Routes } from "react-router-dom";
import { TravelPlanner } from "./components/TravelPlanner";

const App: React.FC = () => {
  return (
    <Routes>
      <Route path="/" element={<TravelPlanner />} />
    </Routes>
  );
};

export default App;
