import React from 'react';
import InfiniteCanvas from '../components/InfiniteCanvas';
import Toolbar from '../components/Toolbar';

const CanvasView: React.FC = () => {
  return (
    <div className="w-screen h-screen bg-gray-50">
      <InfiniteCanvas className="w-full h-full" />
      <Toolbar />
    </div>
  );
};

export default CanvasView;
