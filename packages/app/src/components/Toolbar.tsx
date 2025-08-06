import React from 'react';

const Toolbar: React.FC = () => {
  const handleDragStart = (e: React.DragEvent) => {
    e.dataTransfer.setData('application/tonk-agent', 'tonk');
    e.dataTransfer.effectAllowed = 'copy';
  };

  return (
    <div className="fixed bottom-0 left-0 right-0 bg-white border-t border-gray-200 shadow-lg z-50">
      <div className="flex items-center justify-center p-4">
        <div
          draggable
          onDragStart={handleDragStart}
          className="bg-blue-500 hover:bg-blue-600 text-white px-6 py-3 rounded-lg shadow-md cursor-grab active:cursor-grabbing transition-colors duration-200 select-none"
        >
          Tonk
        </div>
      </div>
    </div>
  );
};

export default Toolbar;
