export interface WidgetProps {
  id: string;
  x: number;
  y: number;
  width?: number;
  height?: number;
  onMove?: (id: string, deltaX: number, deltaY: number) => void;
  onResize?: (id: string, width: number, height: number) => void;
  selected?: boolean;
  data?: Record<string, any>;
}

export interface WidgetDefinition {
  id: string;
  name: string;
  description: string;
  component: React.ComponentType<WidgetProps>;
  defaultProps?: Partial<WidgetProps>;
  icon?: string;
  category?: string;
}

export interface WidgetRegistry {
  [key: string]: WidgetDefinition;
}

export const widgetRegistry: WidgetRegistry = {};

export function registerWidget(definition: WidgetDefinition) {
  widgetRegistry[definition.id] = definition;
}

export function getWidget(id: string): WidgetDefinition | undefined {
  return widgetRegistry[id];
}

export function getAllWidgets(): WidgetDefinition[] {
  return Object.values(widgetRegistry);
}

export async function loadGeneratedWidget(widgetId: string): Promise<WidgetDefinition | null> {
  try {
    // Fetch the compiled widget code
    const response = await fetch(`/api/widgets/compiled/${widgetId}`);
    
    if (!response.ok) {
      console.error(`Widget ${widgetId} not found or compilation failed`);
      return null;
    }

    const code = await response.text();
    
    // Create a blob URL for the compiled module
    const blob = new Blob([code], { type: 'application/javascript' });
    const url = URL.createObjectURL(blob);
    
    try {
      // Import the module from the blob URL
      const module = await import(url);
      return module.default as WidgetDefinition;
    } finally {
      // Clean up the blob URL
      URL.revokeObjectURL(url);
    }
  } catch (error) {
    console.error(`Failed to load widget ${widgetId}:`, error);
    return null;
  }
}
