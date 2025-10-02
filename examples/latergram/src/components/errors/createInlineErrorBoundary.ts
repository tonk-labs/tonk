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
      this.state = {
        hasError: false,
        error: null,
        errorInfo: null,
        errorSent: false,
      };
      this.handleSendToAgent = this.handleSendToAgent.bind(this);
    }

    static getDerivedStateFromError(error: any) {
      return { hasError: true, error };
    }

    componentDidCatch(error: any, errorInfo: any) {
      const context = viewPath ? `[View: ${viewPath}]` : `[${componentName}]`;
      console.error(`${context} Error:`, error, errorInfo);
      this.setState({ errorInfo });
    }

    handleSendToAgent() {
      const state = this.state as any;
      if (state.error && state.errorInfo) {
        this.setState({ errorSent: true });
        sendErrorToAgent(state.error, state.errorInfo, componentName, viewPath);
      }
    }

    render() {
      if ((this.state as any).hasError) {
        const state = this.state as any;
        return buildErrorUI(
          React,
          state.error,
          componentName,
          isPageError,
          viewPath,
          this.handleSendToAgent,
          state.errorSent
        );
      }

      return (this.props as any).children;
    }
  };
}
