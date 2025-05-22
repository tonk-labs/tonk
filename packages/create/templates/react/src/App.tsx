import React, { useEffect } from "react";
import { Route, Routes } from "react-router-dom";
import { HelloWorld } from "./views";
import { stopWorkers } from "./utils/workers";

const App: React.FC = () => {
  // Clean up workers when the app is unmounted
  useEffect(() => {
    return () => {
      // Stop workers when the app is unmounted
      stopWorkers();
    };
  }, []);

  return (
    <Routes>
      <Route path="/" element={<HelloWorld />} />
    </Routes>
  );
};

export default App;
