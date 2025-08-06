import React, { useRef, useCallback, useEffect } from 'react';
import { WidgetProps } from '../index';

interface BaseWidgetProps extends WidgetProps {
  children: React.ReactNode;
  className?: string;
  title?: string;
  backgroundColor?: string;
  borderColor?: string;
}

const BaseWidget: React.FC<BaseWidgetProps> = ({
  id,
  x,
  y,
  width = 200,
  height = 150,
  onMove,
  selected = false,
  children,
  className = '',
  title,
  backgroundColor = 'bg-white',
  borderColor = 'border-gray-200',
}) => {
  const dragRef = useRef<{ isDragging: boolean; lastX: number; lastY: number }>({
    isDragging: false,
    lastX: 0,
    lastY: 0,
  });

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest('.widget-content')) {
      return;
    }

    e.stopPropagation();
    dragRef.current = {
      isDragging: true,
      lastX: e.clientX,
      lastY: e.clientY,
    };
  }, []);

  const handleMouseMove = useCallback(
    (e: MouseEvent) => {
      if (!dragRef.current.isDragging || !onMove) return;

      const deltaX = e.clientX - dragRef.current.lastX;
      const deltaY = e.clientY - dragRef.current.lastY;

      onMove(id, deltaX, deltaY);

      dragRef.current.lastX = e.clientX;
      dragRef.current.lastY = e.clientY;
    },
    [id, onMove]
  );

  const handleMouseUp = useCallback(() => {
    dragRef.current.isDragging = false;
  }, []);

  useEffect(() => {
    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);

    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    };
  }, [handleMouseMove, handleMouseUp]);

  return (
    <div
      className={`absolute ${backgroundColor} rounded-lg shadow-xl border select-none ${
        selected ? 'border-blue-500 border-2' : borderColor
      } ${className}`}
      style={{
        left: x,
        top: y,
        transform: 'translate(-50%, -50%)',
        width: `${width}px`,
        height: `${height}px`,
      }}
    >
      {title && (
        <div
          className="bg-gray-100 text-gray-800 px-4 py-2 rounded-t-lg cursor-grab active:cursor-grabbing flex items-center justify-between border-b"
          onMouseDown={handleMouseDown}
        >
          <span className="font-medium text-sm">{title}</span>
        </div>
      )}
      
      <div
        className="widget-content flex flex-col h-full"
        style={{ height: title ? 'calc(100% - 40px)' : '100%' }}
        onWheel={e => e.stopPropagation()}
      >
        {children}
      </div>
    </div>
  );
};

export default BaseWidget;