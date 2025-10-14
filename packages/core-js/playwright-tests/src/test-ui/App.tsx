import { useState, useEffect } from 'react';
import { getVFSService } from './vfs-service';

interface ServerConfig {
  port: number;
  wsUrl: string;
  manifestUrl: string;
}

interface BrowserMetrics {
  heapUsed: number;
  heapTotal: number;
  heapLimit: number;
  indexedDBSize: number;
}

interface TestState {
  connected: boolean;
  serverConfig: ServerConfig;
  operations: number;
  throughput: number;
  errors: number;
  memoryUsage: number;
  browserMetrics: BrowserMetrics;
  operationTimings: number[];
  startTime: number;
}

// Helper function to get server configuration
function getServerConfig(): ServerConfig {
  // Check for injected config first (from tests)
  const windowConfig = (window as any).serverConfig;
  if (windowConfig) {
    console.log('Using injected server config:', windowConfig);
    return windowConfig;
  }

  // Check URL parameters
  const params = new URLSearchParams(window.location.search);
  const port = params.get('port') || '8081';
  const portNum = parseInt(port, 10);

  const config = {
    port: portNum,
    wsUrl: `ws://localhost:${portNum}`,
    manifestUrl: `http://localhost:${portNum}/.manifest.tonk`,
  };

  console.log('Using URL/default server config:', config);
  return config;
}

function App() {
  const [state, setState] = useState<TestState>(() => {
    const serverConfig = getServerConfig();
    return {
      connected: false,
      serverConfig,
      operations: 0,
      throughput: 0,
      errors: 0,
      memoryUsage: 0,
      browserMetrics: {
        heapUsed: 0,
        heapTotal: 0,
        heapLimit: 0,
        indexedDBSize: 0,
      },
      operationTimings: [],
      startTime: Date.now(),
    };
  });

  const [vfs] = useState(() => getVFSService());

  // Browser metrics collection function
  const collectBrowserMetrics = async (): Promise<BrowserMetrics> => {
    const metrics: BrowserMetrics = {
      heapUsed: 0,
      heapTotal: 0,
      heapLimit: 0,
      indexedDBSize: 0,
    };

    // Get JavaScript heap memory
    if (typeof performance !== 'undefined' && 'memory' in performance) {
      const memInfo = (performance as any).memory;
      metrics.heapUsed = memInfo.usedJSHeapSize || 0;
      metrics.heapTotal = memInfo.totalJSHeapSize || 0;
      metrics.heapLimit = memInfo.jsHeapSizeLimit || 0;
    }

    // Get IndexedDB size
    if (
      typeof navigator !== 'undefined' &&
      'storage' in navigator &&
      'estimate' in navigator.storage
    ) {
      try {
        const estimate = await navigator.storage.estimate();
        metrics.indexedDBSize = estimate.usage || 0;
      } catch (error) {
        console.warn('Failed to estimate IndexedDB size:', error);
      }
    }

    return metrics;
  };

  // Expose VFS service to window for testing
  useEffect(() => {
    (window as any).vfsService = vfs;
    console.log('VFS service exposed to window');

    // Subscribe to operation updates
    const unsubscribe = vfs.onOperationComplete(stats => {
      setState(prev => ({
        ...prev,
        operations: stats.totalOperations,
        errors: stats.totalErrors,
        operationTimings: stats.recentTimings,
      }));
    });

    return () => {
      delete (window as any).vfsService;
      unsubscribe();
    };
  }, [vfs]);

  useEffect(() => {
    // Initialize connection using dynamic server config
    const initVFS = async () => {
      try {
        console.log('Initializing VFS with config:', state.serverConfig);
        await vfs.initialize(
          state.serverConfig.manifestUrl,
          state.serverConfig.wsUrl
        );
        setState(prev => ({ ...prev, connected: true }));
        console.log('VFS connection established successfully');
      } catch (error) {
        console.error('Failed to initialize VFS:', error);
        setState(prev => ({ ...prev, connected: false }));
      }
    };

    initVFS();
  }, [vfs, state.serverConfig]);

  const runThroughputTest = async () => {
    if (!vfs.isInitialized()) return;

    try {
      const testPath = '/test-file.json';
      const testData = { test: 'data', timestamp: Date.now() };

      // Check if file exists to determine if we need to create
      const exists = await vfs.exists(testPath);
      await vfs.writeFile(testPath, { content: testData }, !exists);

      // Read it back
      await vfs.readFile(testPath);
    } catch (error) {
      console.error('Test operation failed:', error);
    }
  };

  const updateMetrics = async () => {
    try {
      const browserMetrics = await collectBrowserMetrics();
      const elapsedSeconds = (Date.now() - state.startTime) / 1000;
      const throughput =
        elapsedSeconds > 0 ? state.operations / elapsedSeconds : 0;

      setState(prev => ({
        ...prev,
        browserMetrics,
        throughput,
        memoryUsage: browserMetrics.heapUsed / (1024 * 1024), // MB
      }));
    } catch (error) {
      console.error('Failed to update metrics:', error);
    }
  };

  // Update metrics every second
  useEffect(() => {
    const interval = setInterval(updateMetrics, 1000);
    return () => clearInterval(interval);
  }, []);

  return (
    <div style={{ padding: '20px', fontFamily: 'monospace' }}>
      <h1>Tonk Test Suite</h1>

      <div style={{ marginBottom: '20px' }}>
        <h2>Connection Status</h2>
        <p data-testid="connection-status">
          Status: {state.connected ? 'Connected' : 'Disconnected'}
        </p>
        <p>Server: {state.serverConfig.wsUrl}</p>
        <p data-testid="server-info">Port: {state.serverConfig.port}</p>
      </div>

      <div style={{ marginBottom: '20px' }}>
        <h2>Test Controls</h2>
        <button
          data-testid="throughput-test-btn"
          onClick={runThroughputTest}
          disabled={!state.connected}
          style={{ marginRight: '10px' }}
        >
          Run Throughput Test
        </button>
      </div>

      <div style={{ marginBottom: '20px' }}>
        <h2>Metrics</h2>
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(2, 1fr)',
            gap: '10px',
            marginBottom: '15px',
          }}
        >
          <div>
            <strong>Operations:</strong>
            <span data-testid="operations-count">{state.operations}</span>
          </div>
          <div>
            <strong>Throughput:</strong>
            <span data-testid="throughput-value">
              {state.throughput.toFixed(2)} ops/sec
            </span>
          </div>
          <div>
            <strong>Errors:</strong>
            <span data-testid="error-count">{state.errors}</span>
          </div>
          <div>
            <strong>Memory:</strong>
            <span data-testid="memory-usage">
              {state.memoryUsage.toFixed(2)} MB
            </span>
          </div>
        </div>

        <h3>Browser Memory Details</h3>
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(2, 1fr)',
            gap: '10px',
          }}
        >
          <div>
            <strong>Heap Used:</strong>
            <span data-testid="heap-used">
              {(state.browserMetrics.heapUsed / (1024 * 1024)).toFixed(2)} MB
            </span>
          </div>
          <div>
            <strong>Heap Total:</strong>
            <span data-testid="heap-total">
              {(state.browserMetrics.heapTotal / (1024 * 1024)).toFixed(2)} MB
            </span>
          </div>
          <div>
            <strong>Heap Limit:</strong>
            <span data-testid="heap-limit">
              {(state.browserMetrics.heapLimit / (1024 * 1024)).toFixed(2)} MB
            </span>
          </div>
          <div>
            <strong>IndexedDB Size:</strong>
            <span data-testid="indexeddb-size">
              {(state.browserMetrics.indexedDBSize / (1024 * 1024)).toFixed(2)}{' '}
              MB
            </span>
          </div>
        </div>
      </div>

      <div>
        <h2>Batch Operations</h2>
        <button
          data-testid="batch-operations-btn"
          onClick={async () => {
            for (let i = 0; i < 10; i++) {
              await runThroughputTest();
            }
          }}
          disabled={!state.connected}
        >
          Run 10 Operations
        </button>
      </div>
    </div>
  );
}

export default App;
