import { sync, DocumentId } from '@tonk/keepsync';
import { create } from 'zustand';

export interface Widget {
  id: string;
  x: number;
  y: number;
  type: 'tonk-agent';
}

interface WidgetData {
  widgets: Widget[];
}

interface WidgetState extends WidgetData {
  // Widget management
  addWidget: (x: number, y: number, type: Widget['type']) => string;
  updateWidgetPosition: (id: string, x: number, y: number) => void;
  removeWidget: (id: string) => void;
  getWidget: (id: string) => Widget | undefined;

  // Utilities
  clearAllWidgets: () => void;
}

export const useWidgetStore = create<WidgetState>(
  sync(
    (set, get) => ({
      widgets: [],

      addWidget: (x: number, y: number, type: Widget['type']) => {
        const widgetId = crypto.randomUUID();
        const newWidget: Widget = {
          id: widgetId,
          x,
          y,
          type,
        };

        set(state => ({
          widgets: [...state.widgets, newWidget],
        }));

        return widgetId;
      },

      updateWidgetPosition: (id: string, x: number, y: number) => {
        set(state => ({
          widgets: state.widgets.map(widget =>
            widget.id === id ? { ...widget, x, y } : widget
          ),
        }));
      },

      removeWidget: (id: string) => {
        set(state => ({
          widgets: state.widgets.filter(widget => widget.id !== id),
        }));
      },

      getWidget: (id: string) => {
        return get().widgets.find(w => w.id === id);
      },

      clearAllWidgets: () => {
        set({ widgets: [] });
      },
    }),
    {
      docId: 'widget-positions' as DocumentId,
    }
  )
);
