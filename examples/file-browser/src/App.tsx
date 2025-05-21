import React from "react";
import { Route, Routes } from "react-router-dom";
import { FileBrowser, FileViewer } from "./views";

const App: React.FC = () => {
  return (
    <Routes>
      <Route path="/" element={<FileBrowser />} />
      <Route path="/view" element={<FileViewer />} />
    </Routes>
  );
};

export default App;
