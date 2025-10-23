import { Extension } from '@tiptap/core';
import { Plugin, PluginKey } from '@tiptap/pm/state';
import { Decoration, DecorationSet } from '@tiptap/pm/view';

export interface LineNumbersOptions {
  enabled: boolean;
}

function createDecorations(state: any): DecorationSet {
  const decorations: Decoration[] = [];
  const doc = state.doc;
  let lineNumber = 1;

  doc.descendants((node: any, pos: number) => {
    if (node.isBlock && node.type.name !== 'doc') {
      const decoration = Decoration.widget(pos, () => {
        const span = document.createElement('span');
        span.className = 'line-number';
        span.textContent = String(lineNumber);
        span.contentEditable = 'false';
        return span;
      }, {
        side: -1,
        key: `line-${lineNumber}`,
      });
      decorations.push(decoration);
      lineNumber++;
    }
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
