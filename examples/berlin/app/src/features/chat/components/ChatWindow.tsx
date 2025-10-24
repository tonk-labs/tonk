import { createPortal } from 'react-dom';
import Draggable from 'react-draggable';
import { useChat } from '../stores/chatStore';
import { useChatSync } from '../hooks/useChatSync';
import { ChatHeader } from './ChatHeader';
import { ChatMessageList } from './ChatMessageList';
import { ChatInput } from './ChatInput';
import { Card } from '../../editor/components/tiptap-ui-primitive/card/card';

export function ChatWindow() {
  const { windowState, updateWindowPosition } = useChat();

  // Sync with VFS and presence
  useChatSync();

  if (!windowState.isOpen) return null;

  const handleDragStop = (_e: unknown, data: { x: number; y: number }) => {
    updateWindowPosition(data.x, data.y);
  };

  return createPortal(
    <Draggable
      handle="[data-drag-handle='true']"
      position={windowState.position}
      onStop={handleDragStop}
      bounds="parent"
    >
      <div
        style={{
          position: 'fixed',
          width: windowState.size.width,
          height: windowState.size.height,
          zIndex: 1000,
        }}
      >
        <Card className="h-full flex flex-col shadow-lg border border-border">
          <ChatHeader />
          <ChatMessageList />
          <ChatInput />
        </Card>
      </div>
    </Draggable>,
    document.body
  );
}
