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
  onMove: _onMove, // Rename to indicate it's intentionally unused
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
        className="absolute bg-white rounded-lg shadow-xl border border-gray-200 select-none flex items-center justify-center"
        style={{
          left: x,
          top: y,
          transform: 'translate(-50%, -50%)',
          width: '200px',
          height: '150px',
        }}
      >
        <div className="text-gray-500 text-sm">Loading widget...</div>
      </div>
    );
  }

  if (error || !widgetDefinition) {
    return (
      <div
        className="absolute bg-red-50 rounded-lg shadow-xl border border-red-200 select-none flex items-center justify-center"
        style={{
          left: x,
          top: y,
          transform: 'translate(-50%, -50%)',
          width: '200px',
          height: '150px',
        }}
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
    <WidgetComponentAny
      widget={widget}
      isSelected={selected}
      onUpdate={handleUpdate}
    />
  );
};

export default DynamicWidget;
