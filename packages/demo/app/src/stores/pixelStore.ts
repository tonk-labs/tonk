import { create } from 'zustand';
import { sync } from '../lib/middleware';

export interface Pixel {
  color: string;
}

interface PixelState {
  pixels: Record<string, Pixel>;
  selectedColor: string;
  setPixel: (x: number, y: number, color: string) => void;
  removePixel: (x: number, y: number) => void;
  setSelectedColor: (color: string) => void;
  clearPixels: () => void;
}

export const usePixelStore = create<PixelState>()(
  sync(
    set => ({
      pixels: {},
      selectedColor: '#000000',

      setPixel: (x: number, y: number, color: string) => {
        const key = `${x},${y}`;
        set((state: PixelState) => ({
          pixels: {
            ...state.pixels,
            [key]: { color },
          },
        }));
      },

      removePixel: (x: number, y: number) => {
        const key = `${x},${y}`;
        set((state: PixelState) => {
          const { [key]: removed, ...remainingPixels } = state.pixels;
          return { pixels: remainingPixels };
        });
      },

      setSelectedColor: (color: string) => {
        set({ selectedColor: color });
      },

      clearPixels: () => {
        set({ pixels: {} });
      },
    }),
    {
      path: '/src/stores/pixels.json',
    }
  )
);
