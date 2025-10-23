import { create } from 'zustand';
// import { sync } from '../../../lib/middleware'; // TEMP: Disabled for local-only testing
import type { JSONContent } from '@tiptap/react';

interface EditorState {
  document: JSONContent | null;
  setDocument: (doc: JSONContent) => void;
  clearDocument: () => void;
}

// TEMP: Using plain Zustand without VFS sync due to connection issues
export const useEditorStore = create<EditorState>()(
  set => ({
    document: null,

    setDocument: (doc: JSONContent) => {
      set({ document: doc });
    },

    clearDocument: () => {
      set({ document: null });
    },
  })
);
