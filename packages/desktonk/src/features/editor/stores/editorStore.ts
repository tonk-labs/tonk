// import { sync } from '../../../lib/middleware'; // TEMP: Disabled for local-only testing
import type { JSONContent } from '@tiptap/react';
import { create } from 'zustand';

export type EditorMode = 'richtext' | 'plaintext';

interface EditorState {
  document: JSONContent | null;
  metadata: {
    title: string;
  };

  // Editor mode state
  editorMode: EditorMode;
  isMarkdownFile: boolean;
  rawMarkdownContent: string | null;

  // Existing actions
  setDocument: (doc: JSONContent) => void;
  setTitle: (title: string) => void;
  setMetadata: (metadata: { title: string }) => void;
  clearDocument: () => void;

  // Mode actions
  setEditorMode: (mode: EditorMode) => void;
  setIsMarkdownFile: (isMarkdown: boolean) => void;
  setRawMarkdownContent: (content: string | null) => void;
  resetEditorState: () => void;
}

// TEMP: Using plain Zustand without VFS sync due to connection issues
export const useEditorStore = create<EditorState>()(set => ({
  document: null,
  metadata: {
    title: 'Untitled',
  },

  // Default state
  editorMode: 'richtext',
  isMarkdownFile: false,
  rawMarkdownContent: null,

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

  // Mode setters
  setEditorMode: (mode: EditorMode) => {
    set({ editorMode: mode });
  },

  setIsMarkdownFile: (isMarkdown: boolean) => {
    set({ isMarkdownFile: isMarkdown });
  },

  setRawMarkdownContent: (content: string | null) => {
    set({ rawMarkdownContent: content });
  },

  resetEditorState: () => {
    set({
      document: null,
      metadata: { title: 'Untitled' },
      editorMode: 'richtext',
      isMarkdownFile: false,
      rawMarkdownContent: null,
    });
  },
}));
