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
  const { isLoading } = useDesktopSync();

  // Enable position persistence
  usePositionSync();

  // Set up navigation handler for double-click events
  useEffect(() => {
    setNavigationHandler((path: string) => {
      navigate(path);
    });
  }, [navigate]);

  if (isLoading) {
    return (
      <div className="desktop-container" style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        background: '#1a1a1a',
        color: '#fff'
      }}>
        Loading desktop...
      </div>
    );
  }

  return null;
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
