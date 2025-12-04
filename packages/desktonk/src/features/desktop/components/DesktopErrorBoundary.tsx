import { Component } from 'react';
import type { ErrorInfo, ReactNode } from 'react';

interface Props {
  children: ReactNode;
}

interface State {
  hasError: boolean;
  error: Error | null;
  errorInfo: ErrorInfo | null;
}

/**
 * Error boundary to catch React errors in the Desktop component.
 * Prevents the entire app from crashing if TLDraw or desktop rendering fails.
 */
export class DesktopErrorBoundary extends Component<Props, State> {
  constructor(props: Props) {
    super(props);
    this.state = {
      hasError: false,
      error: null,
      errorInfo: null,
    };
  }

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: ErrorInfo): void {
    console.error('Desktop component error:', error, errorInfo);
    this.setState({
      error,
      errorInfo,
    });
  }

  handleReset = (): void => {
    this.setState({
      hasError: false,
      error: null,
      errorInfo: null,
    });
    // Force reload to reset desktop state
    window.location.reload();
  };

  render(): ReactNode {
    if (this.state.hasError) {
      return (
        <div
          style={{
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            justifyContent: 'center',
            height: '100vh',
            padding: '2rem',
            textAlign: 'center',
            backgroundColor: '#f5f5f5',
          }}
        >
          <h1 style={{ fontSize: '2rem', marginBottom: '1rem', color: '#333' }}>Desktop Error</h1>
          <p style={{ fontSize: '1rem', marginBottom: '2rem', color: '#666', maxWidth: '600px' }}>
            The desktop encountered an error and couldn't render. This might be due to corrupted
            shape data or a TLDraw rendering issue.
          </p>
          {this.state.error && (
            <details
              style={{
                marginBottom: '2rem',
                padding: '1rem',
                backgroundColor: '#fff',
                borderRadius: '4px',
                border: '1px solid #ddd',
                maxWidth: '600px',
                textAlign: 'left',
              }}
            >
              <summary style={{ cursor: 'pointer', fontWeight: 'bold', marginBottom: '0.5rem' }}>
                Error Details
              </summary>
              <pre
                style={{
                  fontSize: '0.875rem',
                  overflow: 'auto',
                  whiteSpace: 'pre-wrap',
                  wordBreak: 'break-word',
                }}
              >
                {this.state.error.toString()}
                {this.state.errorInfo?.componentStack}
              </pre>
            </details>
          )}
          <button
            type="button"
            onClick={this.handleReset}
            style={{
              padding: '0.75rem 2rem',
              fontSize: '1rem',
              backgroundColor: '#007bff',
              color: 'white',
              border: 'none',
              borderRadius: '4px',
              cursor: 'pointer',
              fontWeight: 'bold',
            }}
            onMouseOver={(e) => {
              e.currentTarget.style.backgroundColor = '#0056b3';
            }}
            onFocus={(e) => {
              e.currentTarget.style.backgroundColor = '#0056b3';
            }}
            onMouseOut={(e) => {
              e.currentTarget.style.backgroundColor = '#007bff';
            }}
            onBlur={(e) => {
              e.currentTarget.style.backgroundColor = '#007bff';
            }}
          >
            Reload Desktop
          </button>
        </div>
      );
    }

    return this.props.children;
  }
}
