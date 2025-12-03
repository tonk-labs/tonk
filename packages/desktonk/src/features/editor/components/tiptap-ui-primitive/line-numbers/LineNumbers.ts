import { Extension } from '@tiptap/react';
import { Plugin, PluginKey } from '@tiptap/pm/state';
import { Decoration, DecorationSet } from '@tiptap/pm/view';
import './LineNumbers.css';

export interface LineNumbersOptions {
  enabled: boolean;
}

// biome-ignore lint/suspicious/noExplicitAny: ProseMirror state type is complex
function createDecorations(state: any): DecorationSet {
  const decorations: Decoration[] = [];
  const doc = state.doc;
  let lineNumber = 0;

  // biome-ignore lint/suspicious/noExplicitAny: ProseMirror node type is complex
  doc.descendants((node: any, pos: number) => {
    // Only add line numbers to top-level block nodes (direct children of doc)
    if (node.isBlock && node.type.name !== 'doc') {
      lineNumber++;
      const currentLine = lineNumber; // Capture current value
      const decoration = Decoration.widget(
        pos + 1,
        () => {
          const span = document.createElement('span');
          span.className = 'line-number';
          span.textContent = String(currentLine);
          span.contentEditable = 'false';
          return span;
        },
        {
          side: -1,
          key: `line-${pos}`,
        }
      );
      decorations.push(decoration);
      return false; // Don't descend into child nodes
    }
    return true;
  });

  return DecorationSet.create(doc, decorations);
}

export const LineNumbers = Extension.create<LineNumbersOptions>({
  name: 'lineNumbers',

  addOptions() {
    return {
      enabled: true,
    };
  },

  addProseMirrorPlugins() {
    const { enabled } = this.options;

    return [
      new Plugin({
        key: new PluginKey('lineNumbers'),
        state: {
          init: (_, state) => {
            if (!enabled) return DecorationSet.empty;
            return createDecorations(state);
          },
          apply: (tr, old, _oldState, newState) => {
            if (!enabled) return DecorationSet.empty;
            if (tr.docChanged) {
              return createDecorations(newState);
            }
            return old;
          },
        },
        props: {
          decorations(state) {
            return this.getState(state);
          },
        },
      }),
    ];
  },
});
