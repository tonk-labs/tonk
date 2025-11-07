import { Tldraw, track } from 'tldraw';
import { useNavigate } from 'react-router-dom';
import { useEffect } from 'react';
import 'tldraw/tldraw.css';
import './desktop.css';
import { FileIconUtil } from '../shapes';
import { useDesktopSync, usePositionSync } from '../hooks';
import { setNavigationHandler } from '../utils/navigationHandler';

const customShapeUtils = [FileIconUtil];

const DesktopInner = track(() => {
  const navigate = useNavigate();
  const { isLoading, files } = useDesktopSync();

  // Enable position persistence
  usePositionSync();

  // Set up navigation handler for double-click events
  useEffect(() => {
    setNavigationHandler((path: string) => {
      navigate(path);
    });

    return () => {
      setNavigationHandler(null);
    };
  }, [navigate]);

  if (isLoading) {
    return (
      <div
        className="desktop-container"
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          background: '#1a1a1a',
          color: '#fff',
        }}
      >
        Loading desktop...
      </div>
    );
  }

  // Show empty state overlay when no files exist (but still render TLDraw)
  const emptyStateOverlay = !isLoading && files.length === 0 ? (
    <div
      style={{
        position: 'absolute',
        top: '50%',
        left: '50%',
        transform: 'translate(-50%, -50%)',
        textAlign: 'center',
        color: '#666',
        fontSize: '1.25rem',
        zIndex: 1000,
        pointerEvents: 'none',
      }}
    >
      <div style={{ fontSize: '3rem', marginBottom: '1rem' }}>📂</div>
      <div>Desktop is empty</div>
      <div style={{ fontSize: '1rem', marginTop: '0.5rem', color: '#888' }}>
        Add files to the desktop directory to see them here
      </div>
    </div>
  ) : null;

  return emptyStateOverlay;
});

function Desktop() {
  return (
    <div className="desktop-container">
      <Tldraw
        className="tldraw-container"
        shapeUtils={customShapeUtils}
      >
        <DesktopInner />
      </Tldraw>
    </div>
  );
}

export default Desktop;
