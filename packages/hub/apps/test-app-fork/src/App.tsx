import React from "react";
import { Route, Routes, Link } from "react-router-dom";
import { NotesView, PhotosView } from "./views";

const App: React.FC = () => {
  return (
    <div className="min-h-screen flex flex-col">
      <nav className="bg-blue-600 text-white py-4 px-6">
        <div className="max-w-6xl mx-auto flex justify-between items-center">
          <div className="text-xl font-bold">Tonk App</div>
          <div className="flex space-x-6">
            <Link to="/" className="hover:text-blue-200 transition-colors">Notes</Link>
            <Link to="/photos" className="hover:text-blue-200 transition-colors">Photos</Link>
          </div>
        </div>
      </nav>
      
      <div className="flex-grow">
        <Routes>
          <Route path="/" element={<NotesView />} />
          <Route path="/photos" element={<PhotosView />} />
        </Routes>
      </div>
    </div>
  );
};

export default App;
