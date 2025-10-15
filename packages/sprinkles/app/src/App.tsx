import { useState, useEffect } from 'react';
import './App.css';
import { useVFS } from './hooks/useVFS';
import type { DocumentData } from '@tonk/core';

function App() {
  const { vfs, connectionState, initialize, isReady, isInitializing, error: vfsError } = useVFS();
  const [output, setOutput] = useState<string>('');
  const [fileWatchId, setFileWatchId] = useState<string | null>(null);
  const [watchUpdates, setWatchUpdates] = useState<number>(0);
  const [watchedFilePath, setWatchedFilePath] = useState<string>('/app/watched.txt');

  // Auto-initialize VFS on mount (you can customize these URLs)
  useEffect(() => {
    if (!isReady && !isInitializing && !vfsError) {
      // These URLs should be adjusted based on your setup
      const manifestUrl = 'http://localhost:8081/.manifest.tonk';
      const wsUrl = 'ws://localhost:8081';

      console.log('[App] Auto-initializing VFS...');
      initialize(manifestUrl, wsUrl).catch((err) => {
        console.error('[App] Failed to initialize VFS:', err);
        setOutput(`Failed to initialize VFS: ${err instanceof Error ? err.message : String(err)}`);
      });
    }
  }, [isReady, isInitializing, vfsError, initialize]);

  const addOutput = (text: string) => {
    setOutput((prev) => prev + '\n' + text);
  };

  const clearOutput = () => {
    setOutput('');
  };

  // File Operations
  const handleReadFile = async () => {
    if (!vfs || !isReady) {
      addOutput('Error: VFS not ready');
      return;
    }

    try {
      addOutput('Reading file /app/demo.txt...');
      const data = await vfs.readFile('/app/demo.txt');
      addOutput(`Success! Content: ${JSON.stringify(data.content, null, 2)}`);
      if (data.bytes) {
        addOutput(`Has bytes: ${data.bytes.substring(0, 50)}...`);
      }
    } catch (err) {
      addOutput(`Error: ${err instanceof Error ? err.message : String(err)}`);
    }
  };

  const handleWriteFile = async () => {
    if (!vfs || !isReady) {
      addOutput('Error: VFS not ready');
      return;
    }

    try {
      const timestamp = Date.now();
      const path = `/app/test-${timestamp}.txt`;
      const content = { text: `Hello from Tonk VFS! Created at ${new Date().toISOString()}` };

      addOutput(`Creating file ${path}...`);
      await vfs.writeFile(path, { content }, true);
      addOutput(`Success! File created at ${path}`);
    } catch (err) {
      addOutput(`Error: ${err instanceof Error ? err.message : String(err)}`);
    }
  };

  const handleListDirectory = async () => {
    if (!vfs || !isReady) {
      addOutput('Error: VFS not ready');
      return;
    }

    try {
      addOutput('Listing directory /app...');
      const files = await vfs.listDirectory('/app');
      addOutput(`Found ${Array.isArray(files) ? files.length : 0} items:`);
      if (Array.isArray(files)) {
        files.forEach((file: any) => {
          addOutput(`  - ${file.name || file.path || JSON.stringify(file)}`);
        });
      } else {
        addOutput(JSON.stringify(files, null, 2));
      }
    } catch (err) {
      addOutput(`Error: ${err instanceof Error ? err.message : String(err)}`);
    }
  };

  const handleDeleteFile = async () => {
    if (!vfs || !isReady) {
      addOutput('Error: VFS not ready');
      return;
    }

    try {
      // Try to delete a test file
      const path = '/app/test-delete-me.txt';

      // First, check if it exists
      const exists = await vfs.exists(path);

      if (!exists) {
        // Create it first
        addOutput(`File ${path} doesn't exist, creating it first...`);
        await vfs.writeFile(path, { content: { text: 'Delete me!' } }, true);
        addOutput('File created!');
      }

      addOutput(`Deleting file ${path}...`);
      await vfs.deleteFile(path);
      addOutput(`Success! File deleted.`);
    } catch (err) {
      addOutput(`Error: ${err instanceof Error ? err.message : String(err)}`);
    }
  };

  // File Watcher Operations
  const handleStartWatch = async () => {
    if (!vfs || !isReady) {
      addOutput('Error: VFS not ready');
      return;
    }

    if (fileWatchId) {
      addOutput('Error: Already watching a file. Stop the current watch first.');
      return;
    }

    try {
      addOutput(`Starting watch on ${watchedFilePath}...`);

      // Make sure the file exists first
      const exists = await vfs.exists(watchedFilePath);
      if (!exists) {
        addOutput(`File doesn't exist, creating ${watchedFilePath}...`);
        await vfs.writeFile(
          watchedFilePath,
          { content: { text: 'Watch me! Update me to see real-time changes.' } },
          true
        );
      }

      const watchId = await vfs.watchFile(watchedFilePath, (data: DocumentData) => {
        setWatchUpdates((prev) => prev + 1);
        addOutput(`File changed! New content: ${JSON.stringify(data.content)}`);
      });

      setFileWatchId(watchId);
      addOutput(`Success! Watching ${watchedFilePath} (ID: ${watchId})`);
    } catch (err) {
      addOutput(`Error: ${err instanceof Error ? err.message : String(err)}`);
    }
  };

  const handleStopWatch = async () => {
    if (!vfs || !isReady) {
      addOutput('Error: VFS not ready');
      return;
    }

    if (!fileWatchId) {
      addOutput('Error: No active watch');
      return;
    }

    try {
      addOutput(`Stopping watch (ID: ${fileWatchId})...`);
      await vfs.unwatchFile(fileWatchId);
      setFileWatchId(null);
      addOutput('Success! Watch stopped.');
    } catch (err) {
      addOutput(`Error: ${err instanceof Error ? err.message : String(err)}`);
    }
  };

  const handleUpdateWatchedFile = async () => {
    if (!vfs || !isReady) {
      addOutput('Error: VFS not ready');
      return;
    }

    if (!fileWatchId) {
      addOutput('Error: No file being watched. Start watching first.');
      return;
    }

    try {
      const newContent = {
        text: `Updated at ${new Date().toISOString()}`,
        updateCount: watchUpdates + 1
      };

      addOutput(`Updating ${watchedFilePath}...`);
      await vfs.writeFile(watchedFilePath, { content: newContent }, false);
      addOutput('File updated! Watch callback should trigger...');
    } catch (err) {
      addOutput(`Error: ${err instanceof Error ? err.message : String(err)}`);
    }
  };

  // Byte Operations
  const handleWriteBytes = async () => {
    if (!vfs || !isReady) {
      addOutput('Error: VFS not ready');
      return;
    }

    try {
      const path = '/app/bytes-test.txt';
      const stringData = 'Hello from bytes! 🚀 This is UTF-8 encoded.';

      addOutput(`Writing string as bytes to ${path}...`);
      await vfs.writeStringAsBytes(path, stringData, true);
      addOutput('Success! String written as bytes.');
    } catch (err) {
      addOutput(`Error: ${err instanceof Error ? err.message : String(err)}`);
    }
  };

  const handleReadBytes = async () => {
    if (!vfs || !isReady) {
      addOutput('Error: VFS not ready');
      return;
    }

    try {
      const path = '/app/bytes-test.txt';

      // Check if exists
      const exists = await vfs.exists(path);
      if (!exists) {
        addOutput(`File ${path} doesn't exist. Write bytes first.`);
        return;
      }

      addOutput(`Reading bytes from ${path}...`);
      const stringData = await vfs.readBytesAsString(path);
      addOutput(`Success! Content: ${stringData}`);
    } catch (err) {
      addOutput(`Error: ${err instanceof Error ? err.message : String(err)}`);
    }
  };

  const getConnectionStatusColor = () => {
    switch (connectionState) {
      case 'connected':
        return '#4ade80';
      case 'connecting':
      case 'reconnecting':
        return '#fbbf24';
      case 'disconnected':
        return '#f87171';
      default:
        return '#9ca3af';
    }
  };

  const getConnectionStatusText = () => {
    switch (connectionState) {
      case 'connected':
        return 'Connected';
      case 'connecting':
        return 'Connecting...';
      case 'reconnecting':
        return 'Reconnecting...';
      case 'disconnected':
        return 'Disconnected';
      default:
        return connectionState;
    }
  };

  return (
    <div className="app">
      <header className="app-header">
        <h1>Tonk VFS Lol</h1>
        <div className="connection-status">
          <span
            className="status-indicator"
            style={{ backgroundColor: getConnectionStatusColor() }}
          />
          <span>{getConnectionStatusText()}</span>
        </div>
      </header>

      {vfsError && (
        <div className="error-banner">
          <strong>VFS Error:</strong> {vfsError}
        </div>
      )}

      {isInitializing && (
        <div className="info-banner">
          Initializing VFS...
        </div>
      )}

      <div className="content">
        <section className="controls-section">
          <h2>File Operations</h2>
          <div className="button-group">
            <button onClick={handleReadFile} disabled={!isReady}>
              Read File
            </button>
            <button onClick={handleWriteFile} disabled={!isReady}>
              Write File
            </button>
            <button onClick={handleListDirectory} disabled={!isReady}>
              List Directory
            </button>
            <button onClick={handleDeleteFile} disabled={!isReady}>
              Delete File
            </button>
          </div>
        </section>

        <section className="controls-section">
          <h2>Byte Operations</h2>
          <div className="button-group">
            <button onClick={handleWriteBytes} disabled={!isReady}>
              Write Bytes
            </button>
            <button onClick={handleReadBytes} disabled={!isReady}>
              Read Bytes
            </button>
          </div>
        </section>

        <section className="controls-section">
          <h2>File Watcher</h2>
          <div className="watcher-info">
            <label>
              Path:
              <input
                type="text"
                value={watchedFilePath}
                onChange={(e) => setWatchedFilePath(e.target.value)}
                disabled={!!fileWatchId}
                placeholder="/app/watched.txt"
              />
            </label>
            <div className="watch-stats">
              Status: {fileWatchId ? 'Active' : 'Inactive'} | Updates: {watchUpdates}
            </div>
          </div>
          <div className="button-group">
            <button onClick={handleStartWatch} disabled={!isReady || !!fileWatchId}>
              Start Watch
            </button>
            <button onClick={handleStopWatch} disabled={!isReady || !fileWatchId}>
              Stop Watch
            </button>
            <button onClick={handleUpdateWatchedFile} disabled={!isReady || !fileWatchId}>
              Update Watched File
            </button>
          </div>
        </section>

        <section className="output-section">
          <div className="output-header">
            <h2>Output</h2>
            <button onClick={clearOutput} className="clear-button">
              Clear
            </button>
          </div>
          <pre className="output">{output || 'No output yet. Try some operations!'}</pre>
        </section>
      </div>

      <footer className="app-footer">
        <p>
          Edit <code>src/App.tsx</code> and save to test HMR
        </p>
        <p className="info-text">
          VFS service communicates with the service worker at localhost:4000
        </p>
      </footer>
    </div>
  );
}

export default App;
