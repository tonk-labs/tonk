import { Button } from '../../../components/ui/button/button';
import { useChat } from '../stores/chatStore';

export function ChatHeader() {
  const { toggleWindow } = useChat();

  return (
    <div
      className="px-4 py-3 border-b border-night-200 dark:border-night-700 bg-white dark:bg-night-900 cursor-move select-none"
      data-drag-handle="true"
    >
      <div className="flex items-center justify-between">
        <h3 className="font-mono font-medium text-sm">Chat</h3>
        <div className="flex gap-1">
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={toggleWindow}
            aria-label="Close chat"
          >
            <svg
              width="16"
              height="16"
              viewBox="0 0 16 16"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              aria-hidden="true"
            >
              <title>Close</title>
              <line x1="4" y1="4" x2="12" y2="12" />
              <line x1="12" y1="4" x2="4" y2="12" />
            </svg>
          </Button>
        </div>
      </div>
    </div>
  );
}
