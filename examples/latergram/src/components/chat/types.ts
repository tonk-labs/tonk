export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant' | 'system' | 'tool';
  content: string;
  timestamp: number;
  toolCalls?: ToolCall[];
  toolName?: string;
  toolCallId?: string;
  hidden?: boolean;
}

export interface ToolCall {
  id: string;
  name: string;
  args: any;
  result?: any;
}

export interface ChatMessageProps {
  message: ChatMessage;
  onEdit?: (message: ChatMessage) => void;
  onDelete?: (messageId: string) => void;
  isEditing?: boolean;
  editContent?: string;
  onEditContentChange?: (content: string) => void;
  onSaveEdit?: () => void;
  onCancelEdit?: () => void;
}

export interface ChatInputProps {
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  disabled?: boolean;
  placeholder?: string;
}

export interface ChatInputBarProps {
  onSendMessage: (text: string) => void;
  isLoading: boolean;
  isReady: boolean;
}

export interface ChatHeaderProps {
  isReady: boolean;
  messageCount: number;
  onClear: () => void;
}