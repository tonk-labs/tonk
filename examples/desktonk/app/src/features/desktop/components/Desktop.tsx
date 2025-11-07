import { Tldraw, track } from 'tldraw';
import { useNavigate } from 'react-router-dom';
import { useEffect, useState } from 'react';
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
  if (!isLoading && files.length === 0) {
    return (
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
    );
  }

  return null;
});

function Desktop() {
  // Detect and track theme changes
  const [isDarkMode, setIsDarkMode] = useState(() => {
    return document.documentElement.classList.contains('dark');
  });

  useEffect(() => {
    // Watch for theme changes
    const observer = new MutationObserver(() => {
      setIsDarkMode(document.documentElement.classList.contains('dark'));
    });

    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['class'],
    });

    return () => observer.disconnect();
  }, []);

  return (
    <div className={`desktop-container ${isDarkMode ? 'dark' : ''}`}>
      <Tldraw
        className="tldraw-container"
        shapeUtils={customShapeUtils}
      >
        <DragDropWrapper />
      </Tldraw>
    </div>
  );
}

// Wrapper that connects drag handlers to outer container
function DragDropWrapper() {
  const { isDraggingOver, handleDrop, handleDragOver, handleDragEnter, handleDragLeave } = useFileDrop();

  // Apply handlers to the desktop-container (parent of Tldraw)
  useEffect(() => {
    const container = document.querySelector('.desktop-container') as HTMLElement;
    if (!container) return;

    // Use capture phase to intercept before TLDraw
    const dropHandler = (e: DragEvent) => {
      handleDrop(e as any);
    };
    const dragOverHandler = (e: DragEvent) => {
      handleDragOver(e as any);
    };
    const dragEnterHandler = (e: DragEvent) => {
      handleDragEnter(e as any);
    };
    const dragLeaveHandler = (e: DragEvent) => {
      handleDragLeave(e as any);
    };

    container.addEventListener('drop', dropHandler, true);
    container.addEventListener('dragover', dragOverHandler, true);
    container.addEventListener('dragenter', dragEnterHandler, true);
    container.addEventListener('dragleave', dragLeaveHandler, true);

    // Add dragging-over class
    if (isDraggingOver) {
      container.classList.add('dragging-over');
    } else {
      container.classList.remove('dragging-over');
    }

    return () => {
      container.removeEventListener('drop', dropHandler, true);
      container.removeEventListener('dragover', dragOverHandler, true);
      container.removeEventListener('dragenter', dragEnterHandler, true);
      container.removeEventListener('dragleave', dragLeaveHandler, true);
      container.classList.remove('dragging-over');
    };
  }, [isDraggingOver, handleDrop, handleDragOver, handleDragEnter, handleDragLeave]);

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
    <>
      <DesktopInner />
      {dragOverlay}
    </>
  );
}

export default Desktop;
