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
    const module = await import(`./generated/${widgetId}/index.ts`);
    return module.default as WidgetDefinition;
  } catch (error) {
    console.error(`Failed to load widget ${widgetId}:`, error);
    return null;
  }
}