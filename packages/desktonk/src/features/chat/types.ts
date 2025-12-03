export interface ChatMessage {
  id: string;
  userId: string;
  text: string;
  timestamp: number;
  reactions: ChatReaction[];
}

export interface ChatReaction {
  emoji: string;
  userIds: string[];
}

export interface WindowState {
  isOpen: boolean;
  position: { x: number; y: number };
  size: { width: number; height: number };
}

export interface ChatConfig {
  maxHistory: number; // 400 default, -1 = infinite
}
