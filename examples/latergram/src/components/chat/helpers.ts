export function formatTimestamp(ts: number): string {
  return new Date(ts).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
  });
}

export function truncateContent(content: string, maxLen: number = 100): string {
  if (content.length <= maxLen) return content;
  return content.slice(0, maxLen) + '...';
}

export function detectMarkdown(content: string): boolean {
  const markdownPatterns = [
    /\*\*/, // Bold
    /`/,    // Code
    /#/,    // Headers
    /🔧/,   // Tool emoji
    /✅/,   // Checkmark emoji
    /\[.*\]\(.*\)/, // Links
    /^[-*+]\s/, // Lists
    /^\d+\.\s/, // Numbered lists
    /```/, // Code blocks
  ];

  return markdownPatterns.some(pattern => pattern.test(content));
}

export function getMessageWarning(
  messageId: string,
  messages: Array<{ id: string }>
): string | null {
  const messageIndex = messages.findIndex(m => m.id === messageId);
  const hasFollowingMessages = messageIndex !== -1 && messageIndex < messages.length - 1;

  if (hasFollowingMessages) {
    return 'Editing will delete all messages after this one and regenerate the response';
  }

  return null;
}