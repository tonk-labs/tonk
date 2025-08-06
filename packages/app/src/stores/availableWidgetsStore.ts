import { create } from 'zustand';
import { widgetLoader } from '../services/widgetLoader';

export interface AvailableWidget {
  id: string;
  name: string;
  description: string;
  icon?: string;
  category?: string;
}

interface AvailableWidgetsState {
  // State
  widgets: AvailableWidget[];
  isLoading: boolean;
  error: string | null;
  lastFetched: number | null;

  // Actions
  fetchAvailableWidgets: () => Promise<void>;
  refreshWidgets: () => Promise<void>;
  addWidget: (widget: AvailableWidget) => void;
  clearError: () => void;
}

export const useAvailableWidgetsStore = create<AvailableWidgetsState>(
  (set, get) => ({
    // Initial state
    widgets: [
      {
        id: 'tonk-agent',
        name: 'Tonk Agent',
        description: 'AI coding assistant that can generate widgets',
        icon: '🤖',
        category: 'ai',
      },
    ],
    isLoading: false,
    error: null,
    lastFetched: null,

    // Fetch available widgets from the server
    fetchAvailableWidgets: async () => {
      const state = get();

      // Don't fetch if already loading
      if (state.isLoading) return;

      set({ isLoading: true, error: null });

      try {
        // Fetch widget list from server
        const response = await fetch('/api/widgets/list');
        if (!response.ok) {
          throw new Error(`HTTP error! status: ${response.status}`);
        }

        const { widgets: widgetIds } = await response.json();

        // Load widget definitions for each widget
        const widgetPromises = widgetIds.map(async (widgetId: string) => {
          try {
            const definition = await widgetLoader.loadGeneratedWidget(widgetId);
            if (definition) {
              return {
                id: definition.id,
                name: definition.name,
                description: definition.description,
                icon: definition.icon,
                category: definition.category,
              };
            }
            return null;
          } catch (error) {
            console.warn(`Failed to load widget ${widgetId}:`, error);
            return null;
          }
        });

        const loadedWidgets = await Promise.all(widgetPromises);
        const validWidgets = loadedWidgets.filter(
          (widget): widget is AvailableWidget => widget !== null
        );

        // Combine with built-in widgets (like tonk-agent)
        const builtInWidgets = state.widgets.filter(w => w.id === 'tonk-agent');
        const allWidgets = [...builtInWidgets, ...validWidgets];

        set({
          widgets: allWidgets,
          isLoading: false,
          lastFetched: Date.now(),
        });
      } catch (error) {
        console.error('Failed to fetch available widgets:', error);
        set({
          error:
            error instanceof Error ? error.message : 'Failed to fetch widgets',
          isLoading: false,
        });
      }
    },

    // Refresh widgets (force refetch)
    refreshWidgets: async () => {
      // Clear cache in widget loader
      widgetLoader.clearCache();

      // Fetch fresh data
      await get().fetchAvailableWidgets();
    },

    // Add a new widget to the list (for immediate updates)
    addWidget: (widget: AvailableWidget) => {
      set(state => ({
        widgets: [...state.widgets.filter(w => w.id !== widget.id), widget],
      }));
    },

    // Clear error state
    clearError: () => {
      set({ error: null });
    },
  })
);

// Auto-fetch widgets on store creation
useAvailableWidgetsStore.getState().fetchAvailableWidgets();
