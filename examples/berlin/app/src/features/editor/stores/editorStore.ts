import { StoreBuilder } from '../../../lib/storeBuilder';
import type { JSONContent } from '@tiptap/react';

interface EditorState {
  document: JSONContent | null;
  metadata: {
    title: string;
  };
}

const initialState: EditorState = {
  document: null,
  metadata: {
    title: 'Untitled',
  },
};

export const editorStore = StoreBuilder(initialState, {
  type: 'vfs',
  path: '/stores/editor.json',
});

export const useEditorStore = editorStore.useStore;

const createEditorActions = () => {
  const store = editorStore;

  return {
    setDocument: (doc: JSONContent) => {
      store.set(state => {
        state.document = doc;
      });
    },

    setTitle: (title: string) => {
      store.set(state => {
        state.metadata.title = title;
      });
    },

    setMetadata: (metadata: { title: string }) => {
      store.set(state => {
        state.metadata = metadata;
      });
    },

    clearDocument: () => {
      store.set(state => {
        state.document = null;
        state.metadata = { title: 'Untitled' };
      });
    },
  };
};

export const useEditor = editorStore.createFactory(createEditorActions());
