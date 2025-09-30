import { buildErrorUI } from './errorBoundaryStyles';
import { sendErrorToAgent } from './sendErrorToAgent';

/**
 * Creates an inline error boundary class for use in dynamic contexts
 * where we can't import React components directly.
 * Uses the shared error UI builder to maintain consistency.
 */
export function createInlineErrorBoundary(
  React: any,
  componentName: string,
  viewPath?: string
) {
  const isPageError = !!viewPath;

  return class extends React.Component {
    constructor(props: any) {
      super(props);
      this.state = { hasError: false, error: null };
    }

    static getDerivedStateFromError(error: any) {
      return { hasError: true, error };
    }

    componentDidCatch(error: any, errorInfo: any) {
      const context = viewPath ? `[View: ${viewPath}]` : `[${componentName}]`;
      console.error(`${context} Error:`, error, errorInfo);

      // Send error to AI agent for automatic fixing (only for views)
      sendErrorToAgent(error, errorInfo, componentName, viewPath);
    }

    render() {
      if ((this.state as any).hasError) {
        const error = (this.state as any).error;
        return buildErrorUI(React, error, componentName, isPageError, viewPath);
      }

      return (this.props as any).children;
    }
  };
}
