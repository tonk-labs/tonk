import { useEffect, useRef, useState } from 'react';
import { useCounterStore } from './counter-store';
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

// Helper function to send messages to service worker
function sendMessageToServiceWorker<T = any>(message: any): Promise<T> {
  return new Promise((resolve, reject) => {
    if (!navigator.serviceWorker.controller) {
      reject(new Error('No service worker controller available'));
      return;
    }

    const requestId = message.id || `${message.type}-${Date.now()}`;
    const messageWithId = { ...message, id: requestId };

    const messageHandler = (event: MessageEvent) => {
      if (
        event.data &&
        event.data.type === message.type &&
        event.data.id === requestId
      ) {
        navigator.serviceWorker.removeEventListener('message', messageHandler);

        if (event.data.success) {
          resolve(event.data as T);
        } else {
          reject(new Error(event.data.error || 'Operation failed'));
        }
      }
    };

    navigator.serviceWorker.addEventListener('message', messageHandler);
    navigator.serviceWorker.controller.postMessage(messageWithId);

    // Timeout after 30 seconds
    setTimeout(() => {
      navigator.serviceWorker.removeEventListener('message', messageHandler);
      reject(new Error('Timeout waiting for service worker response'));
    }, 30000);
  });
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
    wsUrl: 'wss://relay.tonk.xyz',
    manifestUrl: 'https://relay.tonk.xyz/.manifest.tonk',
  };

  // const config = {
  //   port: portNum,
  //   wsUrl: 'ws://localhost:8081',
  //   manifestUrl: 'http://localhost:8081/.manifest.tonk',
  // };

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
  const { counter, increment } = useCounterStore();
  const [bundleStatus, setBundleStatus] = useState<string>('');
  const [uploadedBundleId, setUploadedBundleId] = useState<string>('');

  // Refs to prevent double initialization from React StrictMode
  const initializingRef = useRef(false);
  const initializedRef = useRef(false);

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
    (window as any).__vfsService = vfs; // Also expose with test-expected name
    (window as any).__counterStore = useCounterStore; // Expose store for tests
    console.log('VFS service exposed to window');

    return () => {
      delete (window as any).vfsService;
      delete (window as any).__vfsService;
      delete (window as any).__counterStore;
    };
  }, [vfs]);

  // Listen to VFS connection state changes
  useEffect(() => {
    console.log('[TEST-UI] Subscribing to VFS connection state changes');
    const unsubscribe = vfs.onConnectionStateChange(connectionState => {
      console.log('[TEST-UI] VFS connection state changed:', connectionState);
      setState(prev => ({
        ...prev,
        connected: connectionState === 'connected',
      }));
    });

    return () => {
      console.log('[TEST-UI] Unsubscribing from VFS connection state changes');
      unsubscribe();
    };
  }, [vfs]);

  useEffect(() => {
    // Prevent double initialization in React StrictMode
    if (initializedRef.current) {
      console.log(
        '[TEST-UI] VFS already initialized, skipping duplicate initialization'
      );
      return;
    }

    if (initializingRef.current) {
      console.log('[TEST-UI] VFS initialization already in progress, skipping');
      return;
    }

    // Initialize connection using dynamic server config
    const initVFS = async () => {
      initializingRef.current = true;

      try {
        console.log(
          '[TEST-UI] Fetching shared blank-tonk bundle from relay server'
        );

        // Step 1: Fetch the shared blank-tonk bundle from the relay
        // This ensures all clients start with the same root document
        const blankTonkUrl = 'https://relay.tonk.xyz/api/blank-tonk';
        // const blankTonkUrl = 'http://localhost:8081/api/blank-tonk';
        const response = await fetch(blankTonkUrl);

        if (!response.ok) {
          throw new Error(`Failed to fetch blank-tonk: ${response.statusText}`);
        }

        const bundleBytes = await response.arrayBuffer();
        console.log(
          '[TEST-UI] Fetched blank-tonk bundle:',
          bundleBytes.byteLength,
          'bytes'
        );

        // Step 2: Initialize service worker with the shared bundle bytes
        // This creates the Tonk instance from the same root document
        const initResponse = await sendMessageToServiceWorker({
          type: 'initializeFromBytes',
          bundleBytes: bundleBytes,
          wsUrl: state.serverConfig.wsUrl,
        });

        if (!initResponse.success) {
          throw new Error(
            initResponse.error || 'Service worker initialization failed'
          );
        }

        console.log(
          '[TEST-UI] Service worker initialized successfully from shared bundle'
        );

        // Step 3: Now initialize VFS to connect the Tonk instance to the relay
        // Note: We still need to call vfs.initialize() to set up the VFS service,
        // but the actual Tonk initialization already happened in the service worker
        console.log(
          '[TEST-UI] Connecting VFS to relay:',
          state.serverConfig.wsUrl
        );

        console.log('[TEST-UI] About to call vfs.initialize()...');
        await vfs.initialize(
          state.serverConfig.manifestUrl,
          state.serverConfig.wsUrl
        );
        console.log(
          '[TEST-UI] vfs.initialize() completed, checking if initialized:',
          vfs.isInitialized()
        );

        initializedRef.current = true;
        console.log('[TEST-UI] VFS connection established successfully');
      } catch (error) {
        console.error('[TEST-UI] Failed to initialize:', error);
        console.error('[TEST-UI] Error details:', {
          message: error instanceof Error ? error.message : String(error),
          stack: error instanceof Error ? error.stack : undefined,
        });
        // Don't set initializedRef.current = true on error, allow retry
        // Note: connected state is now managed by the VFS connection state listener
      } finally {
        initializingRef.current = false;
        console.log(
          '[TEST-UI] Initialization finally block, vfs.isInitialized():',
          vfs.isInitialized()
        );
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

  // Bundle upload handler
  const handleUploadBundle = async () => {
    if (!vfs.isInitialized()) {
      setBundleStatus('VFS not initialized');
      return;
    }

    try {
      setBundleStatus('Exporting bundle...');

      // Export bundle from VFS
      const bundleBytes = await vfs.exportBundle();

      setBundleStatus('Uploading to server...');

      // Upload to server (convert Uint8Array to proper type for fetch)
      const blob = new Blob([bundleBytes as any], {
        type: 'application/octet-stream',
      });
      const response = await fetch(`https://relay.tonk.xyz/api/bundles`, {
        // const response = await fetch(`http://localhost:8081/api/bundles`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/octet-stream',
        },
        body: blob,
      });

      if (!response.ok) {
        throw new Error(`Upload failed: ${response.statusText}`);
      }

      const result = await response.json();
      const bundleId = result.id;

      // Store bundle ID for tests
      (window as any).uploadedBundleId = bundleId;
      setUploadedBundleId(bundleId);

      setBundleStatus(`Bundle uploaded: ${bundleId}`);
      console.log('Bundle uploaded successfully:', bundleId);
    } catch (error) {
      const errorMsg = `Upload failed: ${error instanceof Error ? error.message : String(error)}`;
      setBundleStatus(errorMsg);
      console.error(errorMsg);
    }
  };

  // Bundle download handler
  const handleDownloadBundle = async () => {
    if (!vfs.isInitialized()) {
      setBundleStatus('VFS not initialized');
      return;
    }

    try {
      // Get bundle ID from test context or use last uploaded
      const targetBundleId =
        (window as any).__targetBundleId || uploadedBundleId;

      if (!targetBundleId) {
        setBundleStatus('No bundle ID specified');
        return;
      }

      setBundleStatus(`Downloading bundle ${targetBundleId}...`);

      // Download from server
      const response = await fetch(
        `https://relay.tonk.xyz/api/bundles/${targetBundleId}/manifest`
        // `http://localhost:8081/api/bundles/${targetBundleId}/manifest`
      );

      if (!response.ok) {
        throw new Error(`Download failed: ${response.statusText}`);
      }

      const bundleBytes = await response.arrayBuffer();

      setBundleStatus('Bundle downloaded, reloading VFS...');

      // Note: For a real implementation, we'd want to properly reload VFS with the downloaded bundle
      // For now, we'll just mark it as downloaded for test purposes
      (window as any).downloadedBundle = {
        id: targetBundleId,
        size: bundleBytes.byteLength,
        timestamp: Date.now(),
      };

      setBundleStatus(`Bundle ${targetBundleId} downloaded successfully`);
      console.log(
        'Bundle downloaded:',
        targetBundleId,
        bundleBytes.byteLength,
        'bytes'
      );
    } catch (error) {
      const errorMsg = `Download failed: ${error instanceof Error ? error.message : String(error)}`;
      setBundleStatus(errorMsg);
      console.error(errorMsg);
    }
  };

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
        <h2>Counter</h2>
        <p data-testid="counter-value">Counter: {counter}</p>
        <button
          data-testid="increment-btn"
          onClick={increment}
          disabled={!state.connected}
          style={{ marginRight: '10px' }}
        >
          Increment
        </button>
      </div>

      <div style={{ marginBottom: '20px' }}>
        <h2>Bundle Operations</h2>
        <button
          data-testid="upload-bundle-btn"
          onClick={handleUploadBundle}
          disabled={!state.connected}
          style={{ marginRight: '10px' }}
        >
          Upload Bundle
        </button>
        <button
          data-testid="download-bundle-btn"
          onClick={handleDownloadBundle}
          disabled={!state.connected}
          style={{ marginRight: '10px' }}
        >
          Download Bundle
        </button>
        <p data-testid="bundle-status">{bundleStatus}</p>
        {uploadedBundleId && (
          <p data-testid="bundle-id">Bundle ID: {uploadedBundleId}</p>
        )}
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
