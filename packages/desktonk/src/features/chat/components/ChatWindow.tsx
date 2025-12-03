import { createPortal } from 'react-dom';
import { useEffect, useRef } from 'react';
import { useChat } from '../stores/chatStore';
import { useDraggable } from '../hooks/useDraggable';
import { ChatHeader } from './ChatHeader';
import { ChatMessageList } from './ChatMessageList';
import { ChatInput } from './ChatInput';
import { Card } from '../../editor/components/tiptap-ui-primitive/card/card';

export function ChatWindow() {
  const { windowState, updateWindowPosition } = useChat();
  const containerRef = useRef<HTMLDivElement>(null);

  // Custom draggable hook (React 19 compatible)
  const { position, handleMouseDown, setPosition } = useDraggable({
    initialPosition: windowState.position,
    onDragEnd: (newPosition) => {
      updateWindowPosition(newPosition.x, newPosition.y);
    },
  });

  // Sync store position with draggable position
  useEffect(() => {
    setPosition(windowState.position);
  }, [windowState.position, setPosition]);

  // Attach drag handler to drag handle element
  useEffect(() => {
    if (!containerRef.current) return;

    const dragHandle = containerRef.current.querySelector('[data-drag-handle="true"]');
    if (!dragHandle) return;

    const handleMouseDownEvent = (e: MouseEvent) => {
      // Convert native MouseEvent to React MouseEvent-compatible object
      handleMouseDown({
        button: e.button,
        clientX: e.clientX,
        clientY: e.clientY,
        preventDefault: () => e.preventDefault(),
      } as React.MouseEvent);
    };

    dragHandle.addEventListener('mousedown', handleMouseDownEvent as EventListener);

    return () => {
      dragHandle.removeEventListener('mousedown', handleMouseDownEvent as EventListener);
    };
  }, [handleMouseDown]);

  if (!windowState.isOpen) return null;

  return createPortal(
    <div
      ref={containerRef}
      style={{
        position: 'fixed',
        left: position.x,
        top: position.y,
        width: windowState.size.width,
        height: windowState.size.height,
        zIndex: 1000,
      }}
    >
      <Card className="h-full flex flex-col shadow-lg border border-night-200 dark:border-night-700 bg-white dark:bg-night-900">
        <ChatHeader />
        <ChatMessageList />
        <ChatInput />
      </Card>
    </div>,
    document.body
  );
}
