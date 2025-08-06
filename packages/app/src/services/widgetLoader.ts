import { WidgetDefinition } from '../../widgets/index';

interface LoadedWidget {
  definition: WidgetDefinition;
  loadedAt: number;
}

class WidgetLoaderService {
  private loadedWidgets: Map<string, LoadedWidget> = new Map();
  private loadingPromises: Map<string, Promise<WidgetDefinition | null>> =
    new Map();

  async loadGeneratedWidget(
    widgetId: string
  ): Promise<WidgetDefinition | null> {
    // Check if already loaded
    const cached = this.loadedWidgets.get(widgetId);
    if (cached) {
      return cached.definition;
    }

    // Check if currently loading
    const existingPromise = this.loadingPromises.get(widgetId);
    if (existingPromise) {
      return existingPromise;
    }

    // Start loading
    const loadPromise = this.performLoad(widgetId);
    this.loadingPromises.set(widgetId, loadPromise);

    try {
      const result = await loadPromise;
      if (result) {
        this.loadedWidgets.set(widgetId, {
          definition: result,
          loadedAt: Date.now(),
        });
      }
      return result;
    } finally {
      this.loadingPromises.delete(widgetId);
    }
  }

  private async performLoad(
    widgetId: string
  ): Promise<WidgetDefinition | null> {
    try {
      // Dynamic import of the generated widget
      const module = await import(
        `../../widgets/generated/${widgetId}/index.ts`
      );

      if (!module.default) {
        console.error(`Widget ${widgetId} does not have a default export`);
        return null;
      }

      const definition = module.default as WidgetDefinition;

      // Validate the widget definition
      if (!this.validateWidgetDefinition(definition)) {
        console.error(`Widget ${widgetId} has invalid definition`);
        return null;
      }

      console.log(`Successfully loaded widget: ${definition.name}`);
      return definition;
    } catch (error) {
      console.error(`Failed to load widget ${widgetId}:`, error);
      return null;
    }
  }

  private validateWidgetDefinition(
    definition: any
  ): definition is WidgetDefinition {
    return (
      definition &&
      typeof definition.id === 'string' &&
      typeof definition.name === 'string' &&
      typeof definition.description === 'string' &&
      typeof definition.component === 'function'
    );
  }

  async getAllGeneratedWidgets(): Promise<WidgetDefinition[]> {
    try {
      // This would ideally come from an API endpoint that lists available widgets
      // For now, we'll try to load known widgets or scan the directory
      const response = await fetch('/api/widgets/list');
      if (!response.ok) {
        console.warn('Could not fetch widget list from server');
        return [];
      }

      const { widgets } = await response.json();
      const loadPromises = widgets.map((widgetId: string) =>
        this.loadGeneratedWidget(widgetId)
      );

      const results = await Promise.all(loadPromises);
      return results.filter(
        (widget): widget is WidgetDefinition => widget !== null
      );
    } catch (error) {
      console.error('Failed to load generated widgets:', error);
      return [];
    }
  }

  getLoadedWidget(widgetId: string): WidgetDefinition | null {
    const cached = this.loadedWidgets.get(widgetId);
    return cached ? cached.definition : null;
  }

  getAllLoadedWidgets(): WidgetDefinition[] {
    return Array.from(this.loadedWidgets.values()).map(w => w.definition);
  }

  clearCache() {
    this.loadedWidgets.clear();
    this.loadingPromises.clear();
  }

  // Hot reload support for development
  async reloadWidget(widgetId: string): Promise<WidgetDefinition | null> {
    this.loadedWidgets.delete(widgetId);
    this.loadingPromises.delete(widgetId);
    return this.loadGeneratedWidget(widgetId);
  }
}

export const widgetLoader = new WidgetLoaderService();
