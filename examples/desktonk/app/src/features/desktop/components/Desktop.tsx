import { Tldraw } from 'tldraw';
import 'tldraw/tldraw.css';
import './desktop.css';

function Desktop() {
  return (
    <div className="desktop-container">
      <Tldraw
        className="tldraw-container"
      />
    </div>
  );
}

export default Desktop;
