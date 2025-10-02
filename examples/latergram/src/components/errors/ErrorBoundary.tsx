import { AlertTriangle } from 'lucide-react';
import React, { Component, type ReactNode } from 'react';
import { errorStyles } from './errorBoundaryStyles';
import { sendErrorToAgent } from './sendErrorToAgent';

interface ErrorBoundaryProps {
  children: ReactNode;
  componentName?: string;
  viewPath?: string;
  isPageError?: boolean;
  className?: string;
  fallback?: (error: Error, errorInfo: React.ErrorInfo) => ReactNode;
}

interface ErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
  errorInfo: React.ErrorInfo | null;
  errorSent: boolean;
}

export class ErrorBoundary extends Component<
  ErrorBoundaryProps,
  ErrorBoundaryState
> {
  constructor(props: ErrorBoundaryProps) {
    super(props);
    this.state = {
      hasError: false,
      error: null,
      errorInfo: null,
      errorSent: false,
    };
  }

  static getDerivedStateFromError(error: Error): Partial<ErrorBoundaryState> {
    return {
      hasError: true,
      error,
    };
  }

  componentDidCatch(error: Error, errorInfo: React.ErrorInfo): void {
    const { componentName = 'Component', viewPath } = this.props;
    const context = viewPath ? `[View: ${viewPath}]` : `[${componentName}]`;
    console.error(`${context} Error:`, error, errorInfo);
    this.setState({
      error,
      errorInfo,
    });
  }

  handleSendToAgent = () => {
    const { error, errorInfo } = this.state;
    const { componentName = 'Component', viewPath } = this.props;

    if (error && errorInfo) {
      this.setState({ errorSent: true });
      sendErrorToAgent(error, errorInfo, componentName, viewPath);
    }
  };

  render() {
    if (this.state.hasError) {
      const { error, errorSent } = this.state;
      const {
        componentName = 'Component',
        viewPath,
        isPageError,
        className = '',
        fallback,
      } = this.props;

      if (fallback && error && this.state.errorInfo) {
        return fallback(error, this.state.errorInfo);
      }

      // Page-level errors get full-screen treatment
      if (isPageError || viewPath) {
        const viewName =
          viewPath?.split('/').pop()?.replace('.tsx', '') || componentName;
        const styles = errorStyles.page;

        return (
          <div className={`${styles.container} ${className}`}>
            <div className={styles.wrapper}>
              <div className={styles.box}>
                <div className={styles.content}>
                  <div className={styles.iconWrapper}>
                    <div className={styles.icon}>
                      <span className={styles.iconText}>!</span>
                    </div>
                  </div>
                  <div className={styles.body}>
                    <h2 className={styles.title}>Page Error: {viewName}</h2>
                    {viewPath && <p className={styles.path}>{viewPath}</p>}
                    <div className={styles.errorBox}>
                      <p className={styles.errorMessage}>
                        {error?.message || 'Unknown error occurred'}
                      </p>
                    </div>
                    <details className={styles.details}>
                      <summary className={styles.summary}>
                        Show stack trace
                      </summary>
                      <pre className={styles.stackTrace}>
                        {error?.stack || 'No stack trace available'}
                      </pre>
                    </details>
                    <button
                      type="button"
                      onClick={this.handleSendToAgent}
                      disabled={errorSent}
                      className={styles.sendButton}
                    >
                      {errorSent
                        ? '✓ Sent to AI Agent'
                        : 'Ask AI to Fix This Error'}
                    </button>
                  </div>
                </div>
              </div>
            </div>
          </div>
        );
      }

      // Component-level errors get inline treatment
      const styles = errorStyles.component;
      return (
        <div className={`${styles.containerWithMargin} ${className}`}>
          <div className={styles.content}>
            <div className={styles.iconWrapper}>
              <div className={styles.icon}>
                <AlertTriangle className="w-5 h-5 text-white" />
              </div>
            </div>
            <div className={styles.body}>
              <h3 className={styles.title}>{componentName} Failed</h3>
              {error && (
                <>
                  <p className={styles.errorMessage}>{error.message}</p>
                  <details className={styles.details}>
                    <summary className={styles.summary}>
                      Show stack trace
                    </summary>
                    <pre className={styles.stackTrace}>{error.stack}</pre>
                  </details>
                  <button
                    type="button"
                    onClick={this.handleSendToAgent}
                    disabled={errorSent}
                    className={styles.sendButton}
                  >
                    {errorSent ? '✓ Sent to AI' : 'Ask AI to Fix'}
                  </button>
                </>
              )}
            </div>
          </div>
        </div>
      );
    }

    return this.props.children;
  }
}

export const withErrorBoundary = <P extends object>(
  Component: React.ComponentType<P>,
  componentName?: string
) => {
  const WrappedComponent = React.forwardRef<any, P>((props, ref) => (
    <ErrorBoundary
      componentName={componentName || Component.displayName || Component.name}
    >
      <Component {...(props as P)} ref={ref} />
    </ErrorBoundary>
  ));

  WrappedComponent.displayName = `withErrorBoundary(${componentName || Component.displayName || Component.name || 'Component'})`;

  return WrappedComponent;
};
