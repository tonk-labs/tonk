// import { sync } from '../../../lib/middleware'; // TEMP: Disabled for local-only testing
import type { JSONContent } from '@tiptap/react';
import { create } from 'zustand';

interface EditorState {
  document: JSONContent | null;
  metadata: {
    title: string;
  };
  setDocument: (doc: JSONContent) => void;
  setTitle: (title: string) => void;
  setMetadata: (metadata: { title: string }) => void;
  clearDocument: () => void;
}

// TEMP: Using plain Zustand without VFS sync due to connection issues
export const useEditorStore = create<EditorState>()((set) => ({
  document: null,
  metadata: {
    title: 'Untitled',
  },

  setDocument: (doc: JSONContent) => {
    set({ document: doc });
  },

  setTitle: (title: string) => {
    set((state) => ({
      metadata: { ...state.metadata, title },
    }));
  },

  setMetadata: (metadata: { title: string }) => {
    set({ metadata });
  },

  clearDocument: () => {
    set({ document: null, metadata: { title: 'Untitled' } });
  },
}));
