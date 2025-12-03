import { useEditor } from '@tiptap/react';
import { useEffect, useMemo } from 'react';
import StarterKit from '@tiptap/starter-kit';
import Link from '@tiptap/extension-link';
import Image from '@tiptap/extension-image';
import TextAlign from '@tiptap/extension-text-align';
// import { TextStyle } from '@tiptap/r';
import { Color } from '@tiptap/extension-color';
import { Highlight } from '@tiptap/extension-highlight';
import TaskList from '@tiptap/extension-task-list';
import TaskItem from '@tiptap/extension-task-item';
import Typography from '@tiptap/extension-typography';
import Superscript from '@tiptap/extension-superscript';
import Subscript from '@tiptap/extension-subscript';
import HorizontalRule from '@tiptap/extension-horizontal-rule';
import { Selection } from '@tiptap/extensions';
import { ImageUploadNode } from '@/features/editor/components/tiptap-node/image-upload-node/image-upload-node-extension';
import { LineNumbers } from './tiptap-ui-primitive/line-numbers/LineNumbers';
import { useEditorStore } from '../stores/editorStore';
import { SimpleEditor } from './tiptap/simple-editor';
import { handleImageUpload } from '@/lib/utils';
import './editor.css';

export function Editor() {
  const { document, setDocument } = useEditorStore();

  const editor = useEditor({
    extensions: [
      // Configure StarterKit to exclude extensions we'll configure separately
      StarterKit.configure({
        horizontalRule: false,
        link: false,
      }),
      HorizontalRule,
      TextAlign.configure({
        types: ['heading', 'paragraph'],
      }),
      TaskList,
      TaskItem.configure({
        nested: true,
      }),
      Highlight.configure({
        multicolor: true,
      }),
      Typography,
      Superscript,
      Subscript,
      Selection,
      LineNumbers,
      Image,
      ImageUploadNode.configure({
        accept: 'image/*',
        maxSize: 5 * 1024 * 1024, // 5MB
        limit: 3,
        upload: handleImageUpload,
      }),
      Link.configure({
        openOnClick: false,
        HTMLAttributes: {
          class: 'text-blue-600 underline',
        },
      }),
      // TextStyle,
      Color,
    ],
    editorProps: {
      attributes: {
        class: 'simple-editor prose prose-sm sm:prose-base focus:outline-none',
      },
    },
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

  const editorArea = useMemo(() => {
    if (!editor) {
      return <div>Loading editor...</div>;
    }
    return <SimpleEditor editor={editor} />;
  }, [editor]);

  return (
    <div id="editor-wrapper">
      <div className="editor-container">
        <div id="editor-paper" />
      </div>

      {editorArea}
    </div>
  );
}
