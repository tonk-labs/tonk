import { useState, useEffect } from 'react';
import { getVFSService } from './vfs-service';
import { imageGenerator } from '../utils/image-generator';
import { MetricsCollector } from '../utils/metrics-collector';

interface ServerConfig {
  port: number;
  wsUrl: string;
  manifestUrl: string;
}

interface TestState {
  connected: boolean;
  serverConfig: ServerConfig;
  operations: number;
  throughput: number;
  errors: number;
  memoryUsage: number;
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
    };
  });

  const [metrics] = useState(() => new MetricsCollector('ui-test'));
  const [vfs] = useState(() => getVFSService());

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

    const endOperation = metrics.startOperation();

    try {
      // Create a test file
      const testData = JSON.stringify({ test: 'data', timestamp: Date.now() });
      try {
        await vfs.writeFile('/test-file.json', testData, true);
      } catch {
        await vfs.writeFile('/test-file.json', testData, false);
      }

      // Read it back
      await vfs.readFile('/test-file.json');

      endOperation();
      setState(prev => ({ ...prev, operations: prev.operations + 1 }));
    } catch (error) {
      console.error('Test operation failed:', error);
      metrics.recordError('operation-failed');
      setState(prev => ({ ...prev, errors: prev.errors + 1 }));
    }
  };

  const runImageTest = async () => {
    if (!vfs.isInitialized()) return;

    const endOperation = metrics.startOperation();

    try {
      // Generate a small test image
      const image = await imageGenerator.generateImage({
        width: 400,
        height: 300,
        sizeInMB: 0.1,
        format: 'jpeg',
      });

      metrics.recordBytes(image.byteLength);

      // Save to VFS
      await vfs.createFileWithBytes(
        '/test-image.jpg',
        { type: 'image' },
        image
      );

      endOperation();
      setState(prev => ({ ...prev, operations: prev.operations + 1 }));
    } catch (error) {
      console.error('Image test failed:', error);
      metrics.recordError('image-test-failed');
      setState(prev => ({ ...prev, errors: prev.errors + 1 }));
    }
  };

  const updateMetrics = async () => {
    try {
      const currentMetrics = await metrics.getMetrics();
      setState(prev => ({
        ...prev,
        throughput: currentMetrics.throughput.operationsPerSecond,
        memoryUsage: currentMetrics.memory.heapUsed / (1024 * 1024), // MB
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
      <h1>Tonk Performance Test Suite</h1>

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
        <button
          data-testid="image-test-btn"
          onClick={runImageTest}
          disabled={!state.connected}
          style={{ marginRight: '10px' }}
        >
          Run Image Test
        </button>
      </div>

      <div style={{ marginBottom: '20px' }}>
        <h2>Metrics</h2>
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(2, 1fr)',
            gap: '10px',
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
