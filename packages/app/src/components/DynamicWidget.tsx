import React, { useState, useEffect } from 'react';
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
        className={`absolute bg-white rounded-lg shadow-xl border select-none flex items-center justify-center ${
          selected ? 'border-blue-500 border-2' : 'border-gray-200'
        }`}
        style={{
          left: x,
          top: y,
          transform: 'translate(-50%, -50%)',
          width: '200px',
          height: '150px',
          zIndex: 20,
        }}
      >
        <div className="text-gray-500 text-sm">Loading widget...</div>
      </div>
    );
  }

  if (error || !widgetDefinition) {
    return (
      <div
        className={`absolute bg-red-50 rounded-lg shadow-xl border select-none flex items-center justify-center ${
          selected ? 'border-blue-500 border-2' : 'border-red-200'
        }`}
        style={{
          left: x,
          top: y,
          transform: 'translate(-50%, -50%)',
          width: '200px',
          height: '150px',
          zIndex: 20,
        }}
      >
        <div className="text-red-600 text-sm text-center p-4">
          <div className="font-medium">Widget Error</div>
          <div className="text-xs mt-1">{error || 'Unknown error'}</div>
        </div>
      </div>
    );
  }

  // Render the actual widget component - BaseWidget will handle positioning and dragging
  const WidgetComponent = widgetDefinition.component;

  return (
    <WidgetComponent
      id={id}
      x={x}
      y={y}
      width={widgetDefinition.defaultProps?.width || 200}
      height={widgetDefinition.defaultProps?.height || 150}
      selected={selected}
      onMove={onMove}
      data={data}
    />
  );
};

export default DynamicWidget;
