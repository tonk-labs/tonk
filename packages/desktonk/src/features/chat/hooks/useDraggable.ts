import { useState, useRef, useCallback, type MouseEvent } from 'react';

export interface Position {
  x: number;
  y: number;
}

export interface UseDraggableOptions {
  initialPosition?: Position;
  onDrag?: (position: Position) => void;
  onDragEnd?: (position: Position) => void;
}

export interface UseDraggableReturn {
  position: Position;
  isDragging: boolean;
  handleMouseDown: (e: MouseEvent) => void;
  setPosition: (position: Position) => void;
}

/**
 * Custom draggable hook compatible with React 19
 * Replaces react-draggable which uses deprecated findDOMNode API
 */
export const useDraggable = ({
  initialPosition = { x: 100, y: 100 },
  onDrag,
  onDragEnd,
}: UseDraggableOptions = {}): UseDraggableReturn => {
  const [position, setPosition] = useState<Position>(initialPosition);
  const [isDragging, setIsDragging] = useState(false);
  const dragStartPos = useRef<Position>({ x: 0, y: 0 });
  const elementStartPos = useRef<Position>(initialPosition);

  const handleMouseMove = useCallback(
    (e: globalThis.MouseEvent) => {
      const deltaX = e.clientX - dragStartPos.current.x;
      const deltaY = e.clientY - dragStartPos.current.y;

      const newPosition = {
        x: elementStartPos.current.x + deltaX,
        y: elementStartPos.current.y + deltaY,
      };

      setPosition(newPosition);
      onDrag?.(newPosition);
    },
    [onDrag]
  );

  const handleMouseUp = useCallback(
    (e: globalThis.MouseEvent) => {
      setIsDragging(false);

      const deltaX = e.clientX - dragStartPos.current.x;
      const deltaY = e.clientY - dragStartPos.current.y;

      const finalPosition = {
        x: elementStartPos.current.x + deltaX,
        y: elementStartPos.current.y + deltaY,
      };

      onDragEnd?.(finalPosition);

      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    },
    [handleMouseMove, onDragEnd]
  );

  const handleMouseDown = useCallback(
    (e: MouseEvent) => {
      // Only drag on left mouse button
      if (e.button !== 0) return;

      setIsDragging(true);
      dragStartPos.current = { x: e.clientX, y: e.clientY };
      elementStartPos.current = position;

      document.addEventListener('mousemove', handleMouseMove);
      document.addEventListener('mouseup', handleMouseUp);

      e.preventDefault();
    },
    [position, handleMouseMove, handleMouseUp]
  );

  return {
    position,
    isDragging,
    handleMouseDown,
    setPosition,
  };
};
