import { agentChatStore } from '../../lib/agent/agent-chat-store';

/**
 * Sends an error to the AI agent for automatic fixing.
 * Only sends errors when NOT in editor routes to avoid interfering with development.
 */
export function sendErrorToAgent(
  error: Error,
  errorInfo: { componentStack?: string },
  componentName: string,
  viewPath?: string
): void {
  // Check if we're in an editor route
  const currentPath = window.location.pathname;

  // Delay to ensure agent is ready
  setTimeout(async () => {
    const chatStore = agentChatStore.getState();

    // Initialize if not ready
    if (!chatStore.isReady) {
      console.log('AI agent not ready, initializing...');
      await chatStore.initialize();

      // Check again after initialization
      if (!chatStore.isReady) {
        console.log('AI agent still not ready to receive error report');
        // Try again in 2 seconds
        setTimeout(
          () => sendErrorToAgent(error, errorInfo, componentName, viewPath),
          2000
        );
        return;
      }
    }

    if (chatStore.isLoading) {
      console.log('AI agent is busy, waiting to send error report...');
      // Try again in 1 second
      setTimeout(
        () => sendErrorToAgent(error, errorInfo, componentName, viewPath),
        1000
      );
      return;
    }

    // Construct error message for AI
    const errorContext = viewPath
      ? `Error in view: ${viewPath}`
      : `Error in component: ${componentName}`;

    const errorMessage = `🚨 Runtime Error Detected:

${errorContext}
Current route: ${currentPath}

Error: ${error.message}

Stack trace:
${error.stack}

Component stack:
${errorInfo.componentStack}

Please analyze and fix this error automatically.`;

    console.log(
      `Sending ${viewPath ? 'view' : 'component'} error to AI agent for automatic fixing...`
    );

    // Send to AI agent
    chatStore
      .sendMessage(errorMessage)
      .then(() => {
        console.log('Error successfully sent to AI agent');
      })
      .catch((err: any) => {
        console.error('Failed to send error to AI agent:', err);
      });
  }, 500); // Initial 500ms delay to let agent initialize
}
