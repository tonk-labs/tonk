import React from "react";
import { Route, Routes } from "react-router-dom";
import { MediaFeed } from "./views";

const App: React.FC = () => {
  return (
    <Routes>
      <Route path="/" element={<MediaFeed />} />
    </Routes>
  );
};

export default App;
