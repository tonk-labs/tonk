import React, { useEffect } from 'react';
import { useAvailableWidgetsStore } from '../stores/availableWidgetsStore';

const Toolbar: React.FC = () => {
  const { widgets, isLoading, error, fetchAvailableWidgets, clearError } =
    useAvailableWidgetsStore();

  useEffect(() => {
    // Fetch widgets on component mount if not already loaded
    if (widgets.length <= 1) {
      // Only tonk-agent by default
      fetchAvailableWidgets();
    }
  }, [fetchAvailableWidgets, widgets.length]);

  const handleDragStart = (e: React.DragEvent, widgetId: string) => {
    e.dataTransfer.setData('application/widget', widgetId);
    e.dataTransfer.effectAllowed = 'copy';
  };

  const handleRefresh = () => {
    clearError();
    fetchAvailableWidgets();
  };

  return (
    <div className="fixed bottom-0 left-0 right-0 bg-white border-t border-gray-200 shadow-lg z-50">
      <div className="flex items-center justify-between p-4">
        <div className="flex items-center space-x-3 overflow-x-auto">
          {widgets.map(widget => (
            <div
              key={widget.id}
              draggable
              onDragStart={e => handleDragStart(e, widget.id)}
              className="bg-blue-500 hover:bg-blue-600 text-white px-4 py-2 rounded-lg shadow-md cursor-grab active:cursor-grabbing transition-colors duration-200 select-none flex items-center space-x-2 min-w-max"
              title={widget.description}
            >
              {widget.icon && <span className="text-lg">{widget.icon}</span>}
              <span className="text-sm font-medium">{widget.name}</span>
            </div>
          ))}

          {isLoading && (
            <div className="bg-gray-300 text-gray-600 px-4 py-2 rounded-lg shadow-md select-none flex items-center space-x-2">
              <div className="animate-spin w-4 h-4 border-2 border-gray-600 border-t-transparent rounded-full"></div>
              <span className="text-sm">Loading...</span>
            </div>
          )}
        </div>

        <div className="flex items-center space-x-2">
          {error && (
            <div
              className="text-red-600 text-xs max-w-xs truncate"
              title={error}
            >
              Error: {error}
            </div>
          )}

          <button
            onClick={handleRefresh}
            className="bg-gray-500 hover:bg-gray-600 text-white px-3 py-2 rounded-lg shadow-md transition-colors duration-200 text-sm"
            title="Refresh widgets"
          >
            🔄
          </button>
        </div>
      </div>
    </div>
  );
};

export default Toolbar;
