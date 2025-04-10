import React, { useEffect } from "react";
import { HashRouter, Routes, Route } from "react-router-dom";
import Welcome from "./views/Welcome";
import Home from "./views/Home";
import { useConfigStore } from "./stores/configStore";
import { useProjectStore } from "./stores/projectStore";

const App: React.FC = () => {
  const { loadConfig } = useConfigStore();

  const config = useConfigStore.getState().config;
  const { startFileWatching, stopFileWatching } = useProjectStore();

  useEffect(() => {
    const fn = async () => {
      await loadConfig();
      startFileWatching();
    };

    fn();

    return () => {
      stopFileWatching();
    };
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
