import React from "react";
import { MapView } from "../components";

const MapWorldView: React.FC = () => {
  return (
    <main className="flex h-screen bg-gray-100">
      <div className="flex-grow h-full relative">
        <MapView />
      </div>
    </main>
  );
};

export default MapWorldView;
