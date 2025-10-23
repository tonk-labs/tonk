import { create } from 'zustand';
import { sync } from '../../../lib/middleware';
import type { JSONContent } from '@tiptap/react';

interface EditorState {
  document: JSONContent | null;
  setDocument: (doc: JSONContent) => void;
  clearDocument: () => void;
}

export const useEditorStore = create<EditorState>()(
  sync(
    set => ({
      document: null,

      setDocument: (doc: JSONContent) => {
        set({ document: doc });
      },

      clearDocument: () => {
        set({ document: null });
      },
    }),
    {
      path: '/src/stores/editor.json',
    }
  )
);
