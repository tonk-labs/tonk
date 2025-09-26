import React, { useState, useEffect } from 'react';
import { getVFSService } from './vfs-service';
import { imageGenerator } from '../utils/image-generator';
import { MetricsCollector } from '../utils/metrics-collector';

interface TestState {
  connected: boolean;
  serverUrl: string;
  operations: number;
  throughput: number;
  errors: number;
  memoryUsage: number;
}

function App() {
  const [state, setState] = useState<TestState>({
    connected: false,
    serverUrl: 'ws://localhost:8081',
    operations: 0,
    throughput: 0,
    errors: 0,
    memoryUsage: 0,
  });

  const [metrics] = useState(() => new MetricsCollector('ui-test'));
  const [vfs] = useState(() => getVFSService());

  useEffect(() => {
    // Initialize connection
    const initVFS = async () => {
      try {
        const manifestUrl = 'http://localhost:8081/.manifest.tonk';
        const wsUrl = 'ws://localhost:8081';

        await vfs.initialize(manifestUrl, wsUrl);
        setState(prev => ({ ...prev, connected: true }));
      } catch (error) {
        console.error('Failed to initialize VFS:', error);
      }
    };

    initVFS();
  }, [vfs]);

  const runThroughputTest = async () => {
    if (!vfs.isInitialized()) return;

    const endOperation = metrics.startOperation();

    try {
      // Create a test file
      const testData = JSON.stringify({ test: 'data', timestamp: Date.now() });
      await vfs.writeFile('/test-file.json', testData, true);

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
        <p>Server: {state.serverUrl}</p>
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
