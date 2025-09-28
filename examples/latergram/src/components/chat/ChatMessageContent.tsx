import React from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { detectMarkdown } from './helpers';

interface ChatMessageContentProps {
  content: string;
  role: 'user' | 'assistant' | 'system' | 'tool';
}

const ChatMessageContent: React.FC<ChatMessageContentProps> = ({ content, role }) => {
  const hasMarkdown = detectMarkdown(content);

  if (!hasMarkdown) {
    return (
      <p className="text-sm whitespace-pre-wrap break-words">{content}</p>
    );
  }

  return (
    <div className="text-sm prose prose-sm prose-p:font-regular overflow-hidden max-w-full w-full prose-pre:bg-gray-900 prose-pre:text-gray-100 prose-code:text-sm prose-code:bg-gray-100 prose-code:px-1 prose-code:rounded prose-strong:font-semibold prose-p:my-1">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          pre: ({ children, ...props }) => (
            <pre
              className="bg-gray-900 text-gray-100 p-2 rounded-md overflow-x-auto text-[10px] my-2"
              {...props}
            >
              {children}
            </pre>
          ),
          code: ({ className, children, ...props }) => {
            const match = /language-(\w+)/.exec(className || '');
            return match ? (
              <code
                className="block bg-gray-900 text-gray-100 p-2 rounded-md overflow-x-auto text-[10px]"
                {...props}
              >
                {children}
              </code>
            ) : (
              <code
                className="bg-gray-100 px-1 rounded text-[11px]"
                {...props}
              >
                {children}
              </code>
            );
          },
          p: ({ children, ...props }) => (
            <p className="text-sm my-1" {...props}>
              {children}
            </p>
          ),
          strong: ({ children, ...props }) => (
            <strong className="font-semibold" {...props}>
              {children}
            </strong>
          ),
          ul: ({ children, ...props }) => (
            <ul
              className="list-disc list-inside text-sm my-1 pl-2"
              {...props}
            >
              {children}
            </ul>
          ),
          ol: ({ children, ...props }) => (
            <ol
              className="list-decimal list-inside text-sm my-1 pl-2"
              {...props}
            >
              {children}
            </ol>
          ),
          li: ({ children, ...props }) => (
            <li className="text-sm" {...props}>
              {children}
            </li>
          ),
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
};

export default ChatMessageContent;