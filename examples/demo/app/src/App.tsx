import { useEffect, useRef } from 'react';
import './App.css';
import { PixelCanvas } from './components/PixelCanvas';
import { getVFSService } from './lib/vfs-service';

function App() {
  const initRef = useRef(false);

  useEffect(() => {
    if (initRef.current) return;
    initRef.current = true;

    const vfs = getVFSService();

    // Connect to the already-initialized service worker
    // The service worker was initialized by host-web, so this will
    // immediately succeed and trigger the middleware connection listener
    vfs.initialize('', '').catch(err => {
      console.warn('VFS connection warning:', err);
    });
  }, []);

  return (
    <main style={{ width: '100%', height: '100vh', overflow: 'hidden' }}>
      <PixelCanvas />
    </main>
  );
}

export default App;
