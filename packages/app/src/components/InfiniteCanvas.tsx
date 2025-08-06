import React, { useRef, useEffect, useCallback, useState } from 'react';
import { useCanvasStore } from '../stores/canvasStore';
import { useWidgetStore } from '../stores/widgetStore';
import TonkAgent from './TonkAgent';
import DynamicWidget from './DynamicWidget';

interface InfiniteCanvasProps {
  children?: React.ReactNode;
  className?: string;
}

const InfiniteCanvas: React.FC<InfiniteCanvasProps> = ({
  children,
  className = '',
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLDivElement>(null);

  const {
    x,
    y,
    zoom,
    isDragging,
    startDrag,
    updateDrag,
    endDrag,
    zoomAtPoint,
    addAgent,
    moveAgent,
    screenToCanvasCoords,
  } = useCanvasStore();

  const { widgets, removeWidget } = useWidgetStore();

  const [selectedWidgets, setSelectedWidgets] = useState<Set<string>>(
    new Set()
  );
  const [isSelecting, setIsSelecting] = useState(false);
  const [selectionStart, setSelectionStart] = useState<{
    x: number;
    y: number;
  } | null>(null);
  const [selectionEnd, setSelectionEnd] = useState<{
    x: number;
    y: number;
  } | null>(null);

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      const rect = containerRef.current?.getBoundingClientRect();
      if (!rect) return;

      const screenX = e.clientX - rect.left;
      const screenY = e.clientY - rect.top;

      if (e.button === 1) {
        // Middle mouse button - pan the canvas
        e.preventDefault();
        startDrag(e.clientX, e.clientY);
      } else if (e.button === 0) {
        // Left mouse button - start selection
        setIsSelecting(true);
        setSelectionStart({ x: screenX, y: screenY });
        setSelectionEnd({ x: screenX, y: screenY });
      }
    },
    [startDrag]
  );

  const handleMouseMove = useCallback(
    (e: React.MouseEvent) => {
      if (isSelecting) {
        const rect = containerRef.current?.getBoundingClientRect();
        if (rect) {
          const screenX = e.clientX - rect.left;
          const screenY = e.clientY - rect.top;
          setSelectionEnd({ x: screenX, y: screenY });
        }
      } else if (isDragging) {
        updateDrag(e.clientX, e.clientY);
      }
    },
    [isDragging, updateDrag, isSelecting]
  );

  const handleMouseUp = useCallback(() => {
    if (isSelecting && selectionStart && selectionEnd) {
      // Calculate selection rectangle in canvas coordinates
      const startCanvas = screenToCanvasCoords(
        selectionStart.x,
        selectionStart.y
      );
      const endCanvas = screenToCanvasCoords(selectionEnd.x, selectionEnd.y);

      const minX = Math.min(startCanvas.x, endCanvas.x);
      const maxX = Math.max(startCanvas.x, endCanvas.x);
      const minY = Math.min(startCanvas.y, endCanvas.y);
      const maxY = Math.max(startCanvas.y, endCanvas.y);

      // Find widgets within selection rectangle
      const selectedIds = widgets
        .filter(
          widget =>
            widget.x >= minX &&
            widget.x <= maxX &&
            widget.y >= minY &&
            widget.y <= maxY
        )
        .map(widget => widget.id);

      setSelectedWidgets(new Set(selectedIds));

      // Reset selection state
      setIsSelecting(false);
      setSelectionStart(null);
      setSelectionEnd(null);
    } else {
      endDrag();
    }
  }, [
    endDrag,
    isSelecting,
    selectionStart,
    selectionEnd,
    screenToCanvasCoords,
    widgets,
  ]);

  const handleWheel = useCallback(
    (e: React.WheelEvent) => {
      const delta = e.deltaY > 0 ? 0.9 : 1.1;
      const rect = containerRef.current?.getBoundingClientRect();
      if (rect) {
        const pointX = e.clientX - rect.left;
        const pointY = e.clientY - rect.top;
        zoomAtPoint(zoom * delta, pointX, pointY);
      }
    },
    [zoom, zoomAtPoint]
  );

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      const data = e.dataTransfer.getData('application/tonk-agent');
      if (data === 'tonk') {
        const rect = containerRef.current?.getBoundingClientRect();
        if (rect) {
          const screenX = e.clientX - rect.left;
          const screenY = e.clientY - rect.top;
          const canvasCoords = screenToCanvasCoords(screenX, screenY);
          addAgent(canvasCoords.x, canvasCoords.y);
        }
      }
    },
    [addAgent, screenToCanvasCoords]
  );

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = 'copy';
  }, []);

  const handleAgentMove = useCallback(
    (id: string, deltaX: number, deltaY: number) => {
      const scaledDeltaX = deltaX / zoom;
      const scaledDeltaY = deltaY / zoom;
      moveAgent(id, scaledDeltaX, scaledDeltaY);
    },
    [moveAgent, zoom]
  );

  useEffect(() => {
    const handleGlobalMouseUp = () => {
      if (isSelecting) {
        handleMouseUp();
      } else {
        endDrag();
      }
    };

    const handleGlobalMouseMove = (e: MouseEvent) => {
      if (isSelecting) {
        const rect = containerRef.current?.getBoundingClientRect();
        if (rect) {
          const screenX = e.clientX - rect.left;
          const screenY = e.clientY - rect.top;
          setSelectionEnd({ x: screenX, y: screenY });
        }
      } else if (isDragging) {
        updateDrag(e.clientX, e.clientY);
      }
    };

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Delete' || e.key === 'Backspace') {
        selectedWidgets.forEach(widgetId => {
          removeWidget(widgetId);
        });
        setSelectedWidgets(new Set());
      }
    };

    if (isDragging || isSelecting) {
      document.addEventListener('mouseup', handleGlobalMouseUp);
      document.addEventListener('mousemove', handleGlobalMouseMove);
    }

    document.addEventListener('keydown', handleKeyDown);

    return () => {
      document.removeEventListener('mouseup', handleGlobalMouseUp);
      document.removeEventListener('mousemove', handleGlobalMouseMove);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [
    isDragging,
    isSelecting,
    updateDrag,
    endDrag,
    handleMouseUp,
    selectedWidgets,
    removeWidget,
  ]);

  const transform = `translate(${x}px, ${y}px) scale(${zoom})`;

  return (
    <div
      ref={containerRef}
      className={`relative overflow-hidden ${isDragging ? 'cursor-grabbing' : isSelecting ? 'cursor-crosshair' : 'cursor-default'} ${className}`}
      onMouseDown={handleMouseDown}
      onMouseMove={handleMouseMove}
      onMouseUp={handleMouseUp}
      onWheel={handleWheel}
      onDrop={handleDrop}
      onDragOver={handleDragOver}
      onContextMenu={e => e.preventDefault()}
      style={{ width: '100%', height: '100%', touchAction: 'none' }}
    >
      <div
        ref={canvasRef}
        className="absolute inset-0 origin-center"
        style={{
          transform,
          transformOrigin: '0 0',
        }}
      >
        <div className="relative">
          <GridBackground />
          {children}
          {widgets.map(widget => {
            if (widget.type === 'tonk-agent') {
              return (
                <TonkAgent
                  key={widget.id}
                  id={widget.id}
                  x={widget.x}
                  y={widget.y}
                  onMove={handleAgentMove}
                  selected={selectedWidgets.has(widget.id)}
                />
              );
            } else {
              // Render dynamic widgets (AI-generated)
              return (
                <DynamicWidget
                  key={widget.id}
                  widgetId={widget.type}
                  id={widget.id}
                  x={widget.x}
                  y={widget.y}
                  onMove={handleAgentMove}
                  selected={selectedWidgets.has(widget.id)}
                  data={widget.data}
                />
              );
            }
          })}
          {isSelecting && selectionStart && selectionEnd && (
            <div
              className="absolute border-2 border-blue-500 bg-blue-200 bg-opacity-20 pointer-events-none"
              style={{
                left:
                  Math.min(selectionStart.x, selectionEnd.x) / zoom - x / zoom,
                top:
                  Math.min(selectionStart.y, selectionEnd.y) / zoom - y / zoom,
                width: Math.abs(selectionEnd.x - selectionStart.x) / zoom,
                height: Math.abs(selectionEnd.y - selectionStart.y) / zoom,
              }}
            />
          )}
        </div>
      </div>
    </div>
  );
};

const GridBackground: React.FC = () => {
  return (
    <div className="absolute inset-0 pointer-events-none">
      <svg
        width="100%"
        height="100%"
        className="absolute inset-0"
        style={{
          width: '200vw',
          height: '200vh',
          left: '-50vw',
          top: '-50vh',
        }}
      >
        <defs>
          <pattern
            id="grid"
            width="50"
            height="50"
            patternUnits="userSpaceOnUse"
          >
            <path
              d="M 50 0 L 0 0 0 50"
              fill="none"
              stroke="#e5e7eb"
              strokeWidth="1"
            />
          </pattern>
        </defs>
        <rect width="100%" height="100%" fill="url(#grid)" />
      </svg>
    </div>
  );
};

export default InfiniteCanvas;
