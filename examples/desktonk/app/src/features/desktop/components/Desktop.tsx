import { Tldraw, track } from 'tldraw';
import 'tldraw/tldraw.css';
import './desktop.css';
import { FileIconUtil } from '../shapes';
import { useDesktopSync } from '../hooks';

const customShapeUtils = [FileIconUtil];

const DesktopInner = track(() => {
  const { isLoading } = useDesktopSync();

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
