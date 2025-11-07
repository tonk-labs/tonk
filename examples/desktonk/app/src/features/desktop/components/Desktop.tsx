import { Tldraw, track, getDefaultCdnBaseUrl } from 'tldraw';
import { useNavigate } from 'react-router-dom';
import { useEffect } from 'react';
import 'tldraw/tldraw.css';
import './desktop.css';
import { FileIconUtil } from '../shapes';
import { useDesktopSync, usePositionSync } from '../hooks';
import { setNavigationHandler } from '../utils/navigationHandler';
import { useFileDrop } from '../hooks/useFileDrop';

const customShapeUtils = [FileIconUtil];

// Configure asset URLs to use unpkg CDN instead of the broken cdn.tldraw.com
const TLDRAW_VERSION = '4.1.2';
const UNPKG_BASE = `https://unpkg.com/tldraw@${TLDRAW_VERSION}`;
const assetUrls = {
  fonts: {
    tldraw_draw: `${UNPKG_BASE}/fonts/Shantell_Sans-Informal_Regular.woff2`,
    tldraw_draw_italic: `${UNPKG_BASE}/fonts/Shantell_Sans-Informal_Regular_Italic.woff2`,
    tldraw_draw_bold: `${UNPKG_BASE}/fonts/Shantell_Sans-Informal_Bold.woff2`,
    tldraw_draw_italic_bold: `${UNPKG_BASE}/fonts/Shantell_Sans-Informal_Bold_Italic.woff2`,
    tldraw_mono: `${UNPKG_BASE}/fonts/IBMPlexMono-Medium.woff2`,
    tldraw_mono_italic: `${UNPKG_BASE}/fonts/IBMPlexMono-MediumItalic.woff2`,
    tldraw_mono_bold: `${UNPKG_BASE}/fonts/IBMPlexMono-Bold.woff2`,
    tldraw_mono_italic_bold: `${UNPKG_BASE}/fonts/IBMPlexMono-BoldItalic.woff2`,
    tldraw_sans: `${UNPKG_BASE}/fonts/IBMPlexSans-Medium.woff2`,
    tldraw_sans_italic: `${UNPKG_BASE}/fonts/IBMPlexSans-MediumItalic.woff2`,
    tldraw_sans_bold: `${UNPKG_BASE}/fonts/IBMPlexSans-Bold.woff2`,
    tldraw_sans_italic_bold: `${UNPKG_BASE}/fonts/IBMPlexSans-BoldItalic.woff2`,
    tldraw_serif: `${UNPKG_BASE}/fonts/IBMPlexSerif-Medium.woff2`,
    tldraw_serif_italic: `${UNPKG_BASE}/fonts/IBMPlexSerif-MediumItalic.woff2`,
    tldraw_serif_bold: `${UNPKG_BASE}/fonts/IBMPlexSerif-Bold.woff2`,
    tldraw_serif_italic_bold: `${UNPKG_BASE}/fonts/IBMPlexSerif-BoldItalic.woff2`,
  },
  icons: {
    'tool-pointer': `${UNPKG_BASE}/icons/icon/tool-pointer.svg`,
    'tool-hand': `${UNPKG_BASE}/icons/icon/tool-hand.svg`,
    'tool-draw': `${UNPKG_BASE}/icons/icon/tool-draw.svg`,
    'tool-eraser': `${UNPKG_BASE}/icons/icon/tool-eraser.svg`,
    'tool-arrow': `${UNPKG_BASE}/icons/icon/tool-arrow.svg`,
    'tool-text': `${UNPKG_BASE}/icons/icon/tool-text.svg`,
    'tool-note': `${UNPKG_BASE}/icons/icon/tool-note.svg`,
    'tool-rectangle': `${UNPKG_BASE}/icons/icon/tool-rectangle.svg`,
    'tool-ellipse': `${UNPKG_BASE}/icons/icon/tool-ellipse.svg`,
    'tool-triangle': `${UNPKG_BASE}/icons/icon/tool-triangle.svg`,
    'tool-diamond': `${UNPKG_BASE}/icons/icon/tool-diamond.svg`,
    'tool-hexagon': `${UNPKG_BASE}/icons/icon/tool-hexagon.svg`,
    'tool-cloud': `${UNPKG_BASE}/icons/icon/tool-cloud.svg`,
    'tool-star': `${UNPKG_BASE}/icons/icon/tool-star.svg`,
    'tool-oval': `${UNPKG_BASE}/icons/icon/tool-oval.svg`,
    'tool-trapezoid': `${UNPKG_BASE}/icons/icon/tool-trapezoid.svg`,
    'tool-rhombus': `${UNPKG_BASE}/icons/icon/tool-rhombus.svg`,
    'tool-rhombus-2': `${UNPKG_BASE}/icons/icon/tool-rhombus-2.svg`,
    'tool-pentagon': `${UNPKG_BASE}/icons/icon/tool-pentagon.svg`,
    'tool-octagon': `${UNPKG_BASE}/icons/icon/tool-octagon.svg`,
    'tool-heart': `${UNPKG_BASE}/icons/icon/tool-heart.svg`,
    'tool-arrow-left': `${UNPKG_BASE}/icons/icon/tool-arrow-left.svg`,
    'tool-arrow-up': `${UNPKG_BASE}/icons/icon/tool-arrow-up.svg`,
    'tool-arrow-down': `${UNPKG_BASE}/icons/icon/tool-arrow-down.svg`,
    'tool-arrow-right': `${UNPKG_BASE}/icons/icon/tool-arrow-right.svg`,
    'tool-x-box': `${UNPKG_BASE}/icons/icon/tool-x-box.svg`,
    'tool-check-box': `${UNPKG_BASE}/icons/icon/tool-check-box.svg`,
    'tool-frame': `${UNPKG_BASE}/icons/icon/tool-frame.svg`,
    'tool-laser': `${UNPKG_BASE}/icons/icon/tool-laser.svg`,
    'tool-highlight': `${UNPKG_BASE}/icons/icon/tool-highlight.svg`,
    'tool-media': `${UNPKG_BASE}/icons/icon/tool-media.svg`,
    'tool-line': `${UNPKG_BASE}/icons/icon/tool-line.svg`,
  } as Record<string, string>,
  embedIcons: {
    youtube: `${UNPKG_BASE}/embed-icons/youtube.png`,
    figma: `${UNPKG_BASE}/embed-icons/figma.png`,
    github: `${UNPKG_BASE}/embed-icons/github.png`,
    maps: `${UNPKG_BASE}/embed-icons/maps.png`,
    twitter: `${UNPKG_BASE}/embed-icons/twitter.png`,
    codesandbox: `${UNPKG_BASE}/embed-icons/codesandbox.png`,
    codepen: `${UNPKG_BASE}/embed-icons/codepen.png`,
    scratch: `${UNPKG_BASE}/embed-icons/scratch.png`,
    val_town: `${UNPKG_BASE}/embed-icons/val_town.png`,
    google_calendar: `${UNPKG_BASE}/embed-icons/google_calendar.png`,
    google_slides: `${UNPKG_BASE}/embed-icons/google_slides.png`,
  },
};

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
  // TLDraw is always in light mode (no dark mode support)
  return (
    <div className="desktop-container">
      <Tldraw
        className="tldraw-container"
        shapeUtils={customShapeUtils}
        assetUrls={assetUrls}
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
