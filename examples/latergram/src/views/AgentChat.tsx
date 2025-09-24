import { FormEvent, useCallback, useState, useRef, useEffect } from 'react';
import { Bot, Send, User, Trash2, AlertCircle, Loader2, Wrench } from 'lucide-react';
import { useAgent } from '../lib/agent/use-agent';
// poop
const AgentChat: React.FC = () => {
  const [prompt, setPrompt] = useState('');
  const messagesEndRef = useRef<HTMLDivElement>(null);

  const {
    messages,
    isLoading,
    error,
    isReady,
    sendMessage,
    clearConversation,
  } = useAgent();

  const scrollToBottom = () => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  };

  useEffect(() => {
    scrollToBottom();
  }, [messages]);

  const handleSubmit = useCallback(
    async (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      if (!prompt.trim() || !isReady || isLoading) return;

      await sendMessage(prompt);
      setPrompt('');
    },
    [prompt, sendMessage, isReady, isLoading]
  );

  const handleClearConversation = useCallback(async () => {
    if (window.confirm('Are you sure you want to clear the conversation history?')) {
      await clearConversation();
    }
  }, [clearConversation]);

  const renderMessageContent = (message: typeof messages[0]) => {
    return (
      <>
        <p className="text-xs whitespace-pre-wrap break-words">
          {message.content}
        </p>
        {message.toolCalls && message.toolCalls.length > 0 && (
          <div className="mt-2 space-y-1">
            <div className="text-[10px] text-gray-600 font-semibold">Tool Calls:</div>
            {message.toolCalls.map(tool => (
              <details
                key={tool.id}
                className="bg-gray-50 border border-gray-200 rounded p-1.5"
              >
                <summary className="cursor-pointer text-[10px] text-blue-700 font-medium flex items-center gap-1">
                  <Wrench className="w-2.5 h-2.5 inline" />
                  {tool.name}
                </summary>
                <div className="mt-1 pl-3 space-y-1">
                  <div className="text-[9px] text-gray-600">
                    <strong>Args:</strong>
                    <pre className="bg-white p-1 rounded mt-0.5 overflow-x-auto">
                      {JSON.stringify(tool.args, null, 2)}
                    </pre>
                  </div>
                  {tool.result && (
                    <div className="text-[9px] text-gray-600">
                      <strong>Result:</strong>
                      <pre className="bg-white p-1 rounded mt-0.5 overflow-x-auto max-h-32 overflow-y-auto">
                        {JSON.stringify(tool.result, null, 2)}
                      </pre>
                    </div>
                  )}
                </div>
              </details>
            ))}
          </div>
        )}
      </>
    );
  };

  return (
    <div className="flex flex-col h-full bg-gray-50 overflow-hidden">
      {/* Header */}
      <div className="bg-white border-b border-gray-200 px-4 py-2">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Bot className="w-4 h-4 text-blue-500" />
            <h1 className="text-xs font-semibold text-gray-800">Agent Chat</h1>
            {!isReady && (
              <span className="text-[10px] text-amber-600 flex items-center gap-1">
                <Loader2 className="w-3 h-3 animate-spin" />
                Initializing...
              </span>
            )}
            {isReady && (
              <span className="text-[10px] text-green-600">Ready</span>
            )}
          </div>
          <button
            onClick={handleClearConversation}
            className="p-1 hover:bg-gray-100 rounded transition-colors"
            title="Clear conversation"
            disabled={!isReady || messages.length === 0}
          >
            <Trash2 className="w-3 h-3 text-gray-600" />
          </button>
        </div>
      </div>

      {/* Error Banner */}
      {error && (
        <div className="bg-red-50 border-b border-red-200 px-4 py-2">
          <div className="flex items-center gap-2 text-xs text-red-700">
            <AlertCircle className="w-3 h-3" />
            {error}
          </div>
        </div>
      )}

      {/* Messages Area */}
      <div className="flex-1 overflow-y-auto px-3 py-3 min-h-0">
        <div className="max-w-3xl mx-auto space-y-3">
          {messages.length === 0 ? (
            <div className="text-center text-gray-500 text-xs mt-4">
              {isReady
                ? 'Start a conversation by sending a message below'
                : 'Initializing agent service...'}
            </div>
          ) : (
            messages.map(message => (
              <div
                key={message.id}
                className={`flex ${
                  message.role === 'user' ? 'justify-end' : 'justify-start'
                }`}
              >
                <div
                  className={`flex gap-3 max-w-[70%] ${
                    message.role === 'user' ? 'flex-row-reverse' : 'flex-row'
                  }`}
                >
                  <div
                    className={`flex-shrink-0 w-6 h-6 rounded-full flex items-center justify-center ${
                      message.role === 'user'
                        ? 'bg-blue-500'
                        : 'bg-gray-600'
                    }`}
                  >
                    {message.role === 'user' ? (
                      <User className="w-3 h-3 text-white" />
                    ) : (
                      <Bot className="w-3 h-3 text-white" />
                    )}
                  </div>
                  <div
                    className={`px-3 py-1.5 rounded-lg ${
                      message.role === 'user'
                        ? 'bg-blue-500 text-white'
                        : 'bg-white border border-gray-200'
                    }`}
                  >
                    {renderMessageContent(message)}
                    <p
                      className={`text-[10px] mt-0.5 ${
                        message.role === 'user'
                          ? 'text-blue-100'
                          : 'text-gray-500'
                      }`}
                    >
                      {new Date(message.timestamp).toLocaleTimeString([], {
                        hour: '2-digit',
                        minute: '2-digit',
                      })}
                    </p>
                  </div>
                </div>
              </div>
            ))
          )}

          {isLoading && (
            <div className="flex justify-start">
              <div className="flex gap-3 max-w-[70%]">
                <div className="flex-shrink-0 w-6 h-6 rounded-full bg-gray-600 flex items-center justify-center">
                  <Bot className="w-4 h-4 text-white" />
                </div>
                <div className="px-3 py-1.5 rounded-lg bg-white border border-gray-200">
                  <div className="flex gap-1">
                    <div className="w-1.5 h-1.5 bg-gray-400 rounded-full animate-bounce" />
                    <div className="w-1.5 h-1.5 bg-gray-400 rounded-full animate-bounce delay-100" />
                    <div className="w-1.5 h-1.5 bg-gray-400 rounded-full animate-bounce delay-200" />
                  </div>
                </div>
              </div>
            </div>
          )}

          <div ref={messagesEndRef} />
        </div>
      </div>

      {/* Input Area */}
      <div className="bg-white border-t border-gray-200 px-3 py-2">
        <form onSubmit={handleSubmit} className="max-w-3xl mx-auto">
          <div className="flex gap-2">
            <input
              type="text"
              value={prompt}
              onChange={e => setPrompt(e.target.value)}
              placeholder={
                isReady
                  ? 'Type a message...'
                  : 'Waiting for initialization...'
              }
              className="flex-1 px-3 py-1.5 border border-gray-300 rounded-full focus:outline-none focus:ring-2 focus:ring-blue-500 text-xs disabled:bg-gray-100"
              disabled={!isReady || isLoading}
            />
            <button
              type="submit"
              disabled={!prompt.trim() || !isReady || isLoading}
              className="px-3 py-1.5 bg-blue-500 text-white rounded-full hover:bg-blue-600 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
            >
              <Send className="w-3 h-3" />
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};

export default AgentChat;