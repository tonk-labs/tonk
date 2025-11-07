import { Tldraw, track } from 'tldraw';
import { useNavigate } from 'react-router-dom';
import { useEffect } from 'react';
import 'tldraw/tldraw.css';
import './desktop.css';
import { FileIconUtil } from '../shapes';
import { useDesktopSync, usePositionSync } from '../hooks';
import { setNavigationHandler } from '../utils/navigationHandler';
import { useFileDrop } from '../hooks/useFileDrop';

const customShapeUtils = [FileIconUtil];

const DesktopInner = track(() => {
  const navigate = useNavigate();
  const { isLoading, files } = useDesktopSync();
  const { isDraggingOver, handleDrop, handleDragOver, handleDragEnter, handleDragLeave } = useFileDrop();

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

  // Render drag overlay when dragging files over
  const dragOverlay = isDraggingOver ? (
    <div className="drop-overlay">
      <div className="drop-message">
        <div className="drop-icon">📁</div>
        <div>Drop files to upload</div>
      </div>
    </div>
  ) : null;

  return (
    <div
      className={isDraggingOver ? 'dragging-over-content' : ''}
      onDrop={handleDrop}
      onDragOver={handleDragOver}
      onDragEnter={handleDragEnter}
      onDragLeave={handleDragLeave}
      style={{
        position: 'absolute',
        inset: 0,
        pointerEvents: isDraggingOver ? 'auto' : 'none',
      }}
    >
      {emptyStateOverlay}
      {dragOverlay}
    </div>
  );
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
