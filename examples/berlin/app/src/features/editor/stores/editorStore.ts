import { create } from 'zustand';
import { sync } from '../../../lib/middleware';
import type { JSONContent } from '@tiptap/react';

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

export const useEditorStore = create<EditorState>()(
  sync(
    set => ({
      document: null,
      metadata: {
        title: 'Untitled',
      },

      setDocument: (doc: JSONContent) => {
        set({ document: doc });
      },

      setTitle: (title: string) => {
        set(state => ({
          metadata: { ...state.metadata, title },
        }));
      },

      setMetadata: (metadata: { title: string }) => {
        set({ metadata });
      },

      clearDocument: () => {
        set({ document: null, metadata: { title: 'Untitled' } });
      },
    }),
    { path: '/documents/current.json' }
  )
);
