import {
  type KeyboardEvent,
  useCallback,
  useEffect,
  useRef,
  useState,
} from 'react';

export function useScrollToBottom(deps: any[] = []) {
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const isInitialMount = useRef(true);

  const scrollToBottom = useCallback((instant = false) => {
    messagesEndRef.current?.scrollIntoView({
      behavior: instant ? 'instant' : 'smooth',
    });
  }, []);

  useEffect(() => {
    // Instant scroll on initial mount, smooth scroll for updates
    scrollToBottom(isInitialMount.current);
    isInitialMount.current = false;
  }, [scrollToBottom, ...deps]);

  return {
    scrollRef: messagesEndRef,
    scrollToBottom,
  };
}

export function useMessageEdit(
  onUpdate: (messageId: string, content: string) => Promise<void>
) {
  const [editingMessageId, setEditingMessageId] = useState<string | null>(null);
  const [editContent, setEditContent] = useState('');

  const startEdit = useCallback((messageId: string, content: string) => {
    setEditingMessageId(messageId);
    setEditContent(content);
  }, []);

  const saveEdit = useCallback(async () => {
    if (editingMessageId && editContent.trim()) {
      await onUpdate(editingMessageId, editContent);
      setEditingMessageId(null);
      setEditContent('');
    }
  }, [editingMessageId, editContent, onUpdate]);

  const cancelEdit = useCallback(() => {
    setEditingMessageId(null);
    setEditContent('');
  }, []);

  return {
    editingMessageId,
    editContent,
    setEditContent,
    startEdit,
    saveEdit,
    cancelEdit,
  };
}

export function useKeyboardSubmit(onSubmit: () => void) {
  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLTextAreaElement | HTMLInputElement>) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        onSubmit();
      }
    },
    [onSubmit]
  );

  return { handleKeyDown };
}

export function useAutoResize(ref: React.RefObject<HTMLTextAreaElement>) {
  useEffect(() => {
    const element = ref.current;
    if (!element) return;

    const adjustHeight = () => {
      element.style.height = 'auto';
      element.style.height = `${element.scrollHeight}px`;
    };

    element.addEventListener('input', adjustHeight);
    adjustHeight();

    return () => {
      element.removeEventListener('input', adjustHeight);
    };
  }, [ref]);
}
