import React from "react";
import { Route, Routes } from "react-router-dom";
import { MediaFeed } from "./views/MediaFeed";

const App: React.FC = () => {
  return (
    <Routes>
      <Route path="/" element={<MediaFeed />} />
    </Routes>
  );
};

export default App;
