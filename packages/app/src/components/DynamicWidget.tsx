import React, { useState, useEffect, useRef, useCallback } from 'react';
import { WidgetDefinition } from '../../widgets/index';
import { widgetLoader } from '../services/widgetLoader';

interface DynamicWidgetProps {
  widgetId: string;
  id: string;
  x: number;
  y: number;
  onMove: (id: string, deltaX: number, deltaY: number) => void;
  selected?: boolean;
  data?: Record<string, any>;
}

const DynamicWidget: React.FC<DynamicWidgetProps> = ({
  widgetId,
  id,
  x,
  y,
  onMove,
  selected = false,
  data = {},
}) => {
  const [widgetDefinition, setWidgetDefinition] =
    useState<WidgetDefinition | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const dragRef = useRef<{ isDragging: boolean; lastX: number; lastY: number }>(
    {
      isDragging: false,
      lastX: 0,
      lastY: 0,
    }
  );

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    e.stopPropagation();
    dragRef.current = {
      isDragging: true,
      lastX: e.clientX,
      lastY: e.clientY,
    };
  }, []);

  const handleMouseMove = useCallback(
    (e: MouseEvent) => {
      if (!dragRef.current.isDragging) return;

      const deltaX = e.clientX - dragRef.current.lastX;
      const deltaY = e.clientY - dragRef.current.lastY;

      onMove(id, deltaX, deltaY);

      dragRef.current.lastX = e.clientX;
      dragRef.current.lastY = e.clientY;
    },
    [id, onMove]
  );

  const handleMouseUp = useCallback(() => {
    dragRef.current.isDragging = false;
  }, []);

  useEffect(() => {
    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);

    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    };
  }, [handleMouseMove, handleMouseUp]);

  useEffect(() => {
    let mounted = true;

    const loadWidget = async () => {
      try {
        setLoading(true);
        setError(null);

        const definition = await widgetLoader.loadGeneratedWidget(widgetId);

        if (mounted) {
          if (definition) {
            setWidgetDefinition(definition);
          } else {
            setError(`Widget "${widgetId}" not found or failed to load`);
          }
        }
      } catch (err) {
        if (mounted) {
          setError(
            err instanceof Error ? err.message : 'Failed to load widget'
          );
        }
      } finally {
        if (mounted) {
          setLoading(false);
        }
      }
    };

    loadWidget();

    return () => {
      mounted = false;
    };
  }, [widgetId]);

  if (loading) {
    return (
      <div
        className={`absolute bg-white rounded-lg shadow-xl border select-none flex items-center justify-center cursor-grab active:cursor-grabbing ${
          selected ? 'border-blue-500 border-2' : 'border-gray-200'
        }`}
        style={{
          left: x,
          top: y,
          transform: 'translate(-50%, -50%)',
          width: '200px',
          height: '150px',
          zIndex: 10,
        }}
        onMouseDown={handleMouseDown}
      >
        <div className="text-gray-500 text-sm">Loading widget...</div>
      </div>
    );
  }

  if (error || !widgetDefinition) {
    return (
      <div
        className={`absolute bg-red-50 rounded-lg shadow-xl border select-none flex items-center justify-center cursor-grab active:cursor-grabbing ${
          selected ? 'border-blue-500 border-2' : 'border-red-200'
        }`}
        style={{
          left: x,
          top: y,
          transform: 'translate(-50%, -50%)',
          width: '200px',
          height: '150px',
          zIndex: 10,
        }}
        onMouseDown={handleMouseDown}
      >
        <div className="text-red-600 text-sm text-center p-4">
          <div className="font-medium">Widget Error</div>
          <div className="text-xs mt-1">{error || 'Unknown error'}</div>
        </div>
      </div>
    );
  }

  // Render the actual widget component
  const WidgetComponent = widgetDefinition.component;

  // Create widget object for components that expect { widget, isSelected, onUpdate }
  const widget = {
    id,
    x,
    y,
    width: widgetDefinition.defaultProps?.width || 200,
    height: widgetDefinition.defaultProps?.height || 150,
    data,
  };

  const handleUpdate = (updatedWidget: any) => {
    // Handle widget updates - for now just log
    console.log('Widget updated:', updatedWidget);
  };

  // Cast to any to bypass TypeScript interface mismatch
  const WidgetComponentAny = WidgetComponent as any;

  return (
    <div
      className={`absolute select-none cursor-grab active:cursor-grabbing ${
        selected ? 'ring-2 ring-blue-500' : ''
      }`}
      style={{
        left: x,
        top: y,
        transform: 'translate(-50%, -50%)',
        zIndex: 10,
      }}
      onMouseDown={handleMouseDown}
    >
      <WidgetComponentAny
        widget={widget}
        isSelected={selected}
        onUpdate={handleUpdate}
      />
    </div>
  );
};

export default DynamicWidget;
