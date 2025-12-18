import { Color } from '@tiptap/extension-color';
import Document from '@tiptap/extension-document';
import { Highlight } from '@tiptap/extension-highlight';
import HorizontalRule from '@tiptap/extension-horizontal-rule';
import Image from '@tiptap/extension-image';
import Link from '@tiptap/extension-link';
import Paragraph from '@tiptap/extension-paragraph';
import Subscript from '@tiptap/extension-subscript';
import Superscript from '@tiptap/extension-superscript';
import TaskItem from '@tiptap/extension-task-item';
import TaskList from '@tiptap/extension-task-list';
import Text from '@tiptap/extension-text';
import TextAlign from '@tiptap/extension-text-align';
import Typography from '@tiptap/extension-typography';
import { Selection } from '@tiptap/extensions';
import { Markdown } from '@tiptap/markdown';
import { useEditor } from '@tiptap/react';
import StarterKit from '@tiptap/starter-kit';
import { useEffect, useMemo } from 'react';
import { ImageUploadNode } from '@/features/editor/components/tiptap-node/image-upload-node/image-upload-node-extension';
import { handleImageUpload } from '@/lib/utils';
import { type EditorMode, useEditorStore } from '../stores/editorStore';
import { SimpleEditor } from './tiptap/simple-editor';
import { LineNumbers } from './tiptap-ui-primitive/line-numbers/LineNumbers';
import './editor.css';

interface EditorProps {
  initialContent?: string;
  editorMode?: EditorMode;
  isMarkdownFile?: boolean;
}

// Rich text extensions for markdown files
const getRichTextExtensions = () => [
  StarterKit.configure({
    horizontalRule: false,
    link: false,
  }),
  Markdown.configure({
    markedOptions: {
      breaks: true, // Preserve line breaks (convert \n to <br>)
      gfm: true, // Enable GitHub Flavored Markdown
    },
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
    maxSize: 5 * 1024 * 1024,
    limit: 3,
    upload: handleImageUpload,
  }),
  Link.configure({
    openOnClick: false,
    HTMLAttributes: {
      class: 'text-blue-600 underline',
    },
  }),
  Color,
];

// Minimal extensions for plain text files
const getPlainTextExtensions = () => [Document, Paragraph, Text, LineNumbers];

export function Editor({
  initialContent,
  editorMode = 'richtext',
  isMarkdownFile = false,
}: EditorProps) {
  const { document, setDocument, setRawMarkdownContent } = useEditorStore();

  // Get extensions based on mode
  const extensions = useMemo(
    () =>
      editorMode === 'richtext'
        ? getRichTextExtensions()
        : getPlainTextExtensions(),
    [editorMode]
  );

  // Determine initial content
  const initialEditorContent = useMemo(() => {
    if (document) return document;
    return {
      type: 'doc',
      content: [
        {
          type: 'paragraph',
          content: [],
        },
      ],
    };
  }, [document]);

  const editor = useEditor(
    {
      extensions,
      editorProps: {
        attributes: {
          class:
            editorMode === 'richtext'
              ? 'simple-editor prose prose-sm sm:prose-base focus:outline-none'
              : 'simple-editor font-mono text-sm focus:outline-none plaintext-mode',
        },
      },
      content: initialEditorContent,
      onUpdate: ({ editor: ed }) => {
        const json = ed.getJSON();
        setDocument(json);

        // For markdown files, also store the markdown representation
        if (isMarkdownFile && editorMode === 'richtext') {
          try {
            const markdown = ed.getMarkdown();
            setRawMarkdownContent(markdown);
          } catch {
            // Markdown extension might not be ready yet
          }
        }
      },
    },
    [editorMode] // Re-create editor when mode changes
  );

  // Set initial markdown content after editor is ready
  useEffect(() => {
    if (
      editor &&
      isMarkdownFile &&
      initialContent &&
      editorMode === 'richtext'
    ) {
      // Use setContent with markdown parsing - contentType tells TipTap to parse as markdown
      editor.commands.setContent(initialContent, {
        emitUpdate: false,
        contentType: 'markdown',
      });
      // Store the initial markdown
      setRawMarkdownContent(initialContent);
    }
  }, [
    editor,
    isMarkdownFile,
    initialContent,
    editorMode,
    setRawMarkdownContent,
  ]);

  // Sync remote updates from store to TipTap
  useEffect(() => {
    if (editor && document) {
      const currentContent = JSON.stringify(editor.getJSON());
      const newContent = JSON.stringify(document);

      if (currentContent !== newContent) {
        editor.commands.setContent(document);
      }
    }
  }, [document, editor]);

  const editorArea = useMemo(() => {
    if (!editor) {
      return <div>Loading editor...</div>;
    }
    return <SimpleEditor editor={editor} editorMode={editorMode} />;
  }, [editor, editorMode]);

  return (
    <div id="editor-wrapper">
      <div className="editor-container">
        <div id="editor-paper" />
      </div>

      {editorArea}
    </div>
  );
}
