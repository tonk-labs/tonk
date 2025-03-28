import React from "react";
import { Route, Routes } from "react-router-dom";
import { NotesView } from "./views";

const App: React.FC = () => {
  return (
    <Routes>
      <Route path="/" element={<NotesView />} />
    </Routes>
  );
};

export default App;
