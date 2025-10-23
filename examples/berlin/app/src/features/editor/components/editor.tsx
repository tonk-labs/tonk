import { useEditor } from '@tiptap/react';
import { useEffect } from 'react';
import StarterKit from '@tiptap/starter-kit';
import Image from '@tiptap/extension-image';
import Link from '@tiptap/extension-link';
import TextAlign from '@tiptap/extension-text-align';
import Underline from '@tiptap/extension-underline';
import { TextStyle } from '@tiptap/extension-text-style';
import { Color } from '@tiptap/extension-color';
import { Highlight } from '@tiptap/extension-highlight';
import TaskList from '@tiptap/extension-task-list';
import TaskItem from '@tiptap/extension-task-item';
import { useEditorStore } from '../stores/editorStore';
import { SimpleEditor } from './tiptap/simple-editor';

export function Editor() {
  const { document, setDocument } = useEditorStore();

  const editor = useEditor({
    extensions: [
      StarterKit,
      Image,
      Link.configure({
        openOnClick: false,
        HTMLAttributes: {
          class: 'text-blue-600 underline',
        },
      }),
      TextAlign.configure({
        types: ['heading', 'paragraph'],
      }),
      Underline,
      TextStyle,
      Color,
      Highlight,
      TaskList,
      TaskItem.configure({
        nested: true,
      }),
    ],
    content: document || {
      type: 'doc',
      content: [
        {
          type: 'paragraph',
          content: [
            {
              type: 'text',
              text: 'Start typing to see collaborative editing in action...',
            },
          ],
        },
      ],
    },
    onUpdate: ({ editor }) => {
      const json = editor.getJSON();
      setDocument(json);
    },
  });

  // Sync remote updates from store to TipTap
  useEffect(() => {
    if (editor && document) {
      const currentContent = JSON.stringify(editor.getJSON());
      const newContent = JSON.stringify(document);

      // Only update if content actually changed (prevents infinite loops)
      if (currentContent !== newContent) {
        editor.commands.setContent(document);
      }
    }
  }, [document, editor]);

  if (!editor) {
    return <div>Loading editor...</div>;
  }

  return <SimpleEditor editor={editor} />;
}
