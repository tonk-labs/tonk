import type { JSONContent } from '@tiptap/react';
import { useEffect, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { Button } from '@/components/ui/button/button';
import { ChatWindow, useChat } from '@/features/chat';
import { getMimeType } from '@/features/desktop/utils/mimeResolver';
import { Editor } from '@/features/editor';
import { useEditorStore } from '@/features/editor/stores/editorStore';
import { usePresenceTracking } from '@/features/presence';
import Layout from '@/features/text-editor/components/layout/layout';
import { useVFS } from '@/hooks/useVFS';
import { useEditorVFSSave } from './hooks/useEditorVFSSave';
import './index.css';
import styles from './textEditor.module.css';

function TextEditorApp() {
  const [searchParams] = useSearchParams();
  const navigate = useNavigate();
  const { vfs, connectionState } = useVFS();
  const { setDocument, setTitle } = useEditorStore();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Enable presence tracking
  usePresenceTracking();

  // Chat functionality
  const { toggleWindow, windowState } = useChat();

  // Get file path from URL query params
  const filePath = searchParams.get('file');

  // Auto-save editor changes to VFS
  useEditorVFSSave({
    filePath,
    vfs,
    debounceMs: 1000,
  });

  useEffect(() => {
    const loadFile = async () => {
      if (!filePath) {
        setError('No file specified');
        setLoading(false);
        return;
      }

      // Wait for VFS to be connected
      if (connectionState !== 'connected') {
        return;
      }

      try {
        setLoading(true);
        setError(null);

        // Check if file exists
        const exists = await vfs.exists(filePath);
        if (!exists) {
          setError(`File not found: ${filePath}`);
          setLoading(false);
          return;
        }

        // Read file from VFS
        const documentData = await vfs.readFile(filePath);

        // Extract text content based on how the file was stored
        let text: string;
        if (
          typeof documentData.content === 'object' &&
          documentData.content !== null &&
          'text' in documentData.content
        ) {
          // Desktop file with metadata structure
          // biome-ignore lint/suspicious/noExplicitAny: Dynamic content structure
          text = (documentData.content as any).text;
        } else if (documentData.bytes) {
          // File stored as bytes - decode to string
          const decoder = new TextDecoder();
          const binaryString = atob(documentData.bytes);
          const bytes = new Uint8Array(binaryString.length);
          for (let i = 0; i < binaryString.length; i++) {
            bytes[i] = binaryString.charCodeAt(i);
          }
          text = decoder.decode(bytes);
        } else {
          // Fallback: stringify content
          text =
            typeof documentData.content === 'string'
              ? documentData.content
              : JSON.stringify(documentData.content);
        }

        // Extract filename from path and set as title
        const fileName = filePath.split('/').pop() || 'Untitled';
        setTitle(fileName);

        // Convert text to TipTap JSONContent format
        const mimeType = getMimeType(filePath);

        if (mimeType === 'text/html') {
          // For HTML, create a single paragraph with the content
          const htmlContent: JSONContent = {
            type: 'doc',
            content: [
              {
                type: 'paragraph',
                content: [{ type: 'text', text: text }],
              },
            ],
          };
          setDocument(htmlContent);
        } else {
          // Convert plain text to paragraph nodes to preserve newlines
          const lines = text.split('\n');
          const content: JSONContent = {
            type: 'doc',
            content: lines.map(line => ({
              type: 'paragraph',
              content: line.trim() ? [{ type: 'text', text: line }] : [],
            })),
          };
          setDocument(content);
        }

        setLoading(false);
      } catch (err) {
        console.error('Error loading file:', err);
        setError(err instanceof Error ? err.message : 'Failed to load file');
        setLoading(false);
      }
    };

    loadFile();
  }, [filePath, vfs, connectionState, setDocument, setTitle]);

  // Handle error state
  if (error) {
    return (
      <div className={styles.textEditorContainer}>
        <Layout>
          <div className="flex items-center justify-center h-full">
            <div className="text-center">
              <h2 className="text-xl font-bold text-red-600 mb-4">Error</h2>
              <p className="text-gray-300 mb-6">{error}</p>
              <Button variant="default" onClick={() => navigate('/')}>
                Return to Desktop
              </Button>
            </div>
          </div>
        </Layout>
      </div>
    );
  }

  // Handle loading state
  if (loading || connectionState !== 'connected') {
    return (
      <div className={styles.textEditorContainer}>
        <Layout>
          <div className="flex items-center justify-center h-full">
            <div className="text-center">
              <p className="text-gray-300">
                {connectionState !== 'connected'
                  ? `Connecting to VFS... (${connectionState})`
                  : 'Loading file...'}
              </p>
            </div>
          </div>
        </Layout>
      </div>
    );
  }

  return (
    <div className={styles.textEditorContainer}>
      <Layout>
        <Editor />
      </Layout>

      {/* Intercom-style floating chat button */}
      <Button
        variant="default"
        onClick={toggleWindow}
        className="fixed bottom-6 right-6 h-16 w-16 rounded-full shadow-lg hover:shadow-xl transition-all hover:scale-105 z-50 p-0"
        aria-label={windowState.isOpen ? 'Close chat' : 'Open chat'}
      >
        <svg
          width="32"
          height="32"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
        </svg>
      </Button>

      {/* Chat window */}
      <ChatWindow />
    </div>
  );
}

export default TextEditorApp;
