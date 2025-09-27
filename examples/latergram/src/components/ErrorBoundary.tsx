import React, { Component, ReactNode } from 'react';
import { AlertTriangle } from 'lucide-react';

interface ErrorBoundaryProps {
  children: ReactNode;
  componentName?: string;
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
    console.error('Component Error Boundary caught:', error, errorInfo);
    this.setState({
      error,
      errorInfo,
    });
  }

  render() {
    if (this.state.hasError) {
      const { error, errorInfo } = this.state;
      const { componentName = 'Component', fallback } = this.props;

      if (fallback && error && errorInfo) {
        return fallback(error, errorInfo);
      }

      return (
        <div className="bg-red-50 border-2 border-red-500 rounded-lg p-4 m-2">
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
                  {errorInfo && errorInfo.componentStack && (
                    <details className="text-xs">
                      <summary className="text-red-600 cursor-pointer hover:text-red-800">
                        Show stack trace
                      </summary>
                      <pre className="mt-2 p-2 bg-red-100 rounded text-red-700 overflow-x-auto text-xs">
                        {error.stack}
                      </pre>
                    </details>
                  )}
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