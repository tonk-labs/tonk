import React, { Component, ReactNode } from 'react';
import { AlertTriangle } from 'lucide-react';
import { errorStyles } from './errorBoundaryStyles';

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
}

export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  constructor(props: ErrorBoundaryProps) {
    super(props);
    this.state = {
      hasError: false,
      error: null,
      errorInfo: null,
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

  render() {
    if (this.state.hasError) {
      const { error } = this.state;
      const {
        componentName = 'Component',
        viewPath,
        isPageError,
        className = '',
        fallback
      } = this.props;

      if (fallback && error && this.state.errorInfo) {
        return fallback(error, this.state.errorInfo);
      }

      // Page-level errors get full-screen treatment
      if (isPageError || viewPath) {
        const viewName = viewPath?.split('/').pop()?.replace('.tsx', '') || componentName;

        return (
          <div className={`flex items-center justify-center min-h-screen bg-gray-50 p-8 ${className}`}>
            <div className="w-full max-w-2xl">
              <div className="bg-red-50 border-2 border-red-500 rounded-lg p-6">
                <div className="flex items-start gap-4">
                  <div className="flex-shrink-0">
                    <div className="w-10 h-10 bg-red-500 rounded flex items-center justify-center">
                      <span className="text-white font-bold text-xl">!</span>
                    </div>
                  </div>
                  <div className="flex-1">
                    <h2 className="text-red-800 font-bold text-lg mb-2">
                      Page Error: {viewName}
                    </h2>
                    {viewPath && (
                      <p className="text-red-700 text-sm font-mono mb-3">
                        {viewPath}
                      </p>
                    )}
                    <div className="bg-red-100 rounded p-3 mb-3">
                      <p className="text-red-800 font-mono text-sm">
                        {error?.message || 'Unknown error occurred'}
                      </p>
                    </div>
                    <details className="text-sm">
                      <summary className="text-red-600 cursor-pointer hover:text-red-800 font-medium">
                        Show stack trace
                      </summary>
                      <pre className="mt-3 p-3 bg-white border border-red-200 rounded text-red-700 overflow-x-auto text-xs font-mono">
                        {error?.stack || 'No stack trace available'}
                      </pre>
                    </details>
                  </div>
                </div>
              </div>
            </div>
          </div>
        );
      }

      // Component-level errors get inline treatment
      return (
        <div className={`bg-red-50 border-2 border-red-500 rounded-lg p-4 ${className || 'm-2'}`}>
          <div className="flex items-start gap-3">
            <div className="flex-shrink-0">
              <div className="w-8 h-8 bg-red-500 rounded flex items-center justify-center">
                <AlertTriangle className="w-5 h-5 text-white" />
              </div>
            </div>
            <div className="flex-1 min-w-0">
              <h3 className="text-red-800 font-semibold text-sm mb-1">
                {componentName} Failed
              </h3>
              {error && (
                <>
                  <p className="text-red-700 text-xs font-mono mb-2">
                    {error.message}
                  </p>
                  <details className="text-xs">
                    <summary className="text-red-600 cursor-pointer hover:text-red-800">
                      Show stack trace
                    </summary>
                    <pre className="mt-2 p-2 bg-red-100 rounded text-red-700 overflow-x-auto text-xs">
                      {error.stack}
                    </pre>
                  </details>
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
    <ErrorBoundary componentName={componentName || Component.displayName || Component.name}>
      <Component {...props} ref={ref} />
    </ErrorBoundary>
  ));

  WrappedComponent.displayName = `withErrorBoundary(${componentName || Component.displayName || Component.name || 'Component'})`;

  return WrappedComponent;
};