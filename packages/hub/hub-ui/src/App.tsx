import React, { useEffect } from "react";
import { HashRouter, Routes, Route } from "react-router-dom";
import Welcome from "./views/Welcome";
import Home from "./views/Home";
import { useConfigStore } from "./stores/configStore";

const App: React.FC = () => {
  const { loadConfig } = useConfigStore();

  useEffect(() => {
    loadConfig();
  }, []);

  return (
    <HashRouter>
      <Routes>
        <Route path="/home" element={<Home />} />
        <Route path="/" element={<Welcome />} />
      </Routes>
    </HashRouter>
  );
};

export default App;
