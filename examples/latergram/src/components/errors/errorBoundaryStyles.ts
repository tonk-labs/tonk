/**
 * Shared styling configuration for error boundaries
 * Used by both ErrorBoundary component and inline error boundaries
 */

export const errorStyles = {
  // Page-level error styles
  page: {
    container: 'flex items-center justify-center min-h-screen bg-gray-50 p-8',
    wrapper: 'w-full max-w-2xl',
    box: 'bg-red-50 border-2 border-red-500 rounded-lg p-6',
    content: 'flex items-start gap-4',
    iconWrapper: 'flex-shrink-0',
    icon: 'w-10 h-10 bg-red-500 rounded flex items-center justify-center',
    iconText: 'text-white font-bold text-xl',
    body: 'flex-1',
    title: 'text-red-800 font-bold text-lg mb-2',
    path: 'text-red-700 text-sm font-mono mb-3',
    errorBox: 'bg-red-100 rounded p-3 mb-3',
    errorMessage: 'text-red-800 font-mono text-sm',
    details: 'text-sm',
    summary: 'text-red-600 cursor-pointer hover:text-red-800 font-medium',
    stackTrace:
      'mt-3 p-3 bg-white border border-red-200 rounded text-red-700 overflow-x-auto text-xs font-mono',
    sendButton:
      'mt-4 w-full bg-blue-600 hover:bg-blue-700 disabled:bg-gray-400 disabled:cursor-not-allowed text-white font-semibold py-2 px-4 rounded transition-colors',
  },

  // Component-level error styles
  component: {
    container: 'bg-red-50 border-2 border-red-500 rounded-lg p-4',
    containerWithMargin: 'bg-red-50 border-2 border-red-500 rounded-lg p-4 m-2',
    content: 'flex items-start gap-3',
    iconWrapper: 'flex-shrink-0',
    icon: 'w-8 h-8 bg-red-500 rounded flex items-center justify-center',
    iconText: 'text-white font-bold',
    body: 'flex-1 min-w-0',
    title: 'text-red-800 font-semibold text-sm mb-1',
    errorMessage: 'text-red-700 text-xs font-mono mb-2',
    details: 'text-xs',
    summary: 'text-red-600 cursor-pointer hover:text-red-800',
    stackTrace:
      'mt-2 p-2 bg-red-100 rounded text-red-700 overflow-x-auto text-xs',
    sendButton:
      'mt-2 w-full bg-blue-600 hover:bg-blue-700 disabled:bg-gray-400 disabled:cursor-not-allowed text-white font-semibold text-xs py-1.5 px-3 rounded transition-colors',
  },
};

/**
 * Helper to build error UI structure using React.createElement
 * for contexts where we can't use JSX
 */
export function buildErrorUI(
  React: any,
  error: any,
  componentName: string,
  isPageError: boolean,
  viewPath?: string,
  onSendToAgent?: () => void,
  errorSent?: boolean
) {
  const styles = isPageError ? errorStyles.page : errorStyles.component;
  const displayName =
    viewPath?.split('/').pop()?.replace('.tsx', '') || componentName;

  if (isPageError) {
    const pageStyles = errorStyles.page;
    return React.createElement(
      'div',
      { className: pageStyles.container },
      React.createElement(
        'div',
        { className: pageStyles.wrapper },
        React.createElement(
          'div',
          { className: pageStyles.box },
          React.createElement('div', { className: styles.content }, [
            // Icon
            React.createElement(
              'div',
              { key: 'icon', className: styles.iconWrapper },
              React.createElement(
                'div',
                { className: styles.icon },
                React.createElement('span', { className: styles.iconText }, '!')
              )
            ),
            // Content
            React.createElement(
              'div',
              { key: 'content', className: styles.body },
              [
                React.createElement(
                  'h2',
                  { key: 'title', className: styles.title },
                  `Page Error: ${displayName}`
                ),
                viewPath &&
                  React.createElement(
                    'p',
                    { key: 'path', className: pageStyles.path },
                    viewPath
                  ),
                React.createElement(
                  'div',
                  { key: 'error-box', className: pageStyles.errorBox },
                  React.createElement(
                    'p',
                    { className: styles.errorMessage },
                    error?.message || 'Unknown error occurred'
                  )
                ),
                React.createElement(
                  'details',
                  { key: 'stack', className: styles.details },
                  [
                    React.createElement(
                      'summary',
                      { key: 'summary', className: styles.summary },
                      'Show stack trace'
                    ),
                    React.createElement(
                      'pre',
                      { key: 'trace', className: styles.stackTrace },
                      error?.stack || 'No stack trace available'
                    ),
                  ]
                ),
                onSendToAgent &&
                  React.createElement(
                    'button',
                    {
                      key: 'send-button',
                      type: 'button',
                      onClick: onSendToAgent,
                      disabled: errorSent,
                      className: pageStyles.sendButton,
                    },
                    errorSent
                      ? '✓ Sent to AI Agent'
                      : 'Ask AI to Fix This Error'
                  ),
              ]
            ),
          ])
        )
      )
    );
  }

  // Component error
  const componentStyles = errorStyles.component;
  return React.createElement(
    'div',
    { className: componentStyles.containerWithMargin },
    React.createElement('div', { className: styles.content }, [
      // Icon
      React.createElement(
        'div',
        { key: 'icon', className: styles.iconWrapper },
        React.createElement(
          'div',
          { className: styles.icon },
          React.createElement('span', { className: styles.iconText }, '!')
        )
      ),
      // Content
      React.createElement('div', { key: 'content', className: styles.body }, [
        React.createElement(
          'h3',
          { key: 'title', className: styles.title },
          `${componentName} Failed`
        ),
        React.createElement(
          'p',
          { key: 'message', className: styles.errorMessage },
          error?.message || 'Unknown error'
        ),
        React.createElement(
          'details',
          { key: 'stack', className: styles.details },
          [
            React.createElement(
              'summary',
              { key: 'summary', className: styles.summary },
              'Show stack trace'
            ),
            React.createElement(
              'pre',
              { key: 'trace', className: styles.stackTrace },
              error?.stack || 'No stack trace available'
            ),
          ]
        ),
        onSendToAgent &&
          React.createElement(
            'button',
            {
              key: 'send-button',
              type: 'button',
              onClick: onSendToAgent,
              disabled: errorSent,
              className: componentStyles.sendButton,
            },
            errorSent ? '✓ Sent to AI' : 'Ask AI to Fix'
          ),
      ]),
    ])
  );
}
