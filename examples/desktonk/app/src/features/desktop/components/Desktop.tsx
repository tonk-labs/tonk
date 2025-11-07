import { Tldraw } from 'tldraw';
import 'tldraw/tldraw.css';
import './desktop.css';
import { FileIconUtil } from '../shapes';

const customShapeUtils = [FileIconUtil];

function Desktop() {
  return (
    <div className="desktop-container">
      <Tldraw
        className="tldraw-container"
        shapeUtils={customShapeUtils}
      />
    </div>
  );
}

export default Desktop;
