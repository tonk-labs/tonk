import Layout from './components/layout/layout';
import { Editor } from './features/editor';
import { usePresenceTracking } from './features/presence';
import { ChatWindow, useChat } from './features/chat';
import { Button } from './components/ui/button/button';

function App() {
  // Enable presence tracking
  usePresenceTracking();

  // Chat functionality
  const { toggleWindow, windowState } = useChat();

  return (
    <>
      <Layout>
        <Editor/>
      </Layout>

      {/* Intercom-style floating chat button */}
      <Button
        variant="default"
        onClick={toggleWindow}
        className="fixed bottom-6 right-6 h-16 w-16 rounded-full shadow-lg hover:shadow-xl transition-all hover:scale-105 z-50 p-0"
        aria-label={windowState.isOpen ? "Close chat" : "Open chat"}
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
    </>
  );
}

export default App;
