import { create } from 'zustand';
import { useWidgetStore } from './widgetStore';

interface CanvasState {
  x: number;
  y: number;
  zoom: number;
  isDragging: boolean;
  dragStart: { x: number; y: number } | null;
}

interface CanvasActions {
  setPosition: (x: number, y: number) => void;
  setZoom: (zoom: number) => void;
  zoomAtPoint: (zoom: number, pointX: number, pointY: number) => void;
  startDrag: (x: number, y: number) => void;
  updateDrag: (x: number, y: number) => void;
  endDrag: () => void;
  reset: () => void;
  addAgent: (x: number, y: number) => void;
  moveAgent: (id: string, deltaX: number, deltaY: number) => void;
  removeAgent: (id: string) => void;
  screenToCanvasCoords: (
    screenX: number,
    screenY: number
  ) => { x: number; y: number };
}

type CanvasStore = CanvasState & CanvasActions;

const initialState: CanvasState = {
  x: 0,
  y: 0,
  zoom: 1,
  isDragging: false,
  dragStart: null,
};

export const useCanvasStore = create<CanvasStore>((set, get) => ({
  ...initialState,

  setPosition: (x: number, y: number) => set({ x, y }),

  setZoom: (zoom: number) => {
    const clampedZoom = Math.max(0.1, Math.min(5, zoom));
    set({ zoom: clampedZoom });
  },

  zoomAtPoint: (zoom: number, pointX: number, pointY: number) => {
    const state = get();
    const clampedZoom = Math.max(0.1, Math.min(5, zoom));

    // Calculate the point in canvas coordinates before zoom
    const canvasPointX = (pointX - state.x) / state.zoom;
    const canvasPointY = (pointY - state.y) / state.zoom;

    // Calculate new position to keep the point under cursor
    const newX = pointX - canvasPointX * clampedZoom;
    const newY = pointY - canvasPointY * clampedZoom;

    set({
      zoom: clampedZoom,
      x: newX,
      y: newY,
    });
  },

  startDrag: (x: number, y: number) =>
    set({
      isDragging: true,
      dragStart: { x, y },
    }),

  updateDrag: (x: number, y: number) => {
    const state = get();
    if (state.isDragging && state.dragStart) {
      const deltaX = x - state.dragStart.x;
      const deltaY = y - state.dragStart.y;
      set({
        x: state.x + deltaX,
        y: state.y + deltaY,
        dragStart: { x, y },
      });
    }
  },

  endDrag: () =>
    set({
      isDragging: false,
      dragStart: null,
    }),

  reset: () => set(initialState),

  addAgent: (x: number, y: number) => {
    const { addWidget } = useWidgetStore.getState();
    addWidget(x, y, 'tonk-agent');
  },

  moveAgent: (id: string, deltaX: number, deltaY: number) => {
    const { getWidget, updateWidgetPosition } = useWidgetStore.getState();
    const widget = getWidget(id);
    if (widget) {
      updateWidgetPosition(id, widget.x + deltaX, widget.y + deltaY);
    }
  },

  removeAgent: (id: string) => {
    const { removeWidget } = useWidgetStore.getState();
    removeWidget(id);
  },

  screenToCanvasCoords: (screenX: number, screenY: number) => {
    const state = get();
    return {
      x: (screenX - state.x) / state.zoom,
      y: (screenY - state.y) / state.zoom,
    };
  },
}));
