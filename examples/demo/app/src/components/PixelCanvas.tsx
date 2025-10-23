import { useRef, useEffect, useState } from 'react';
import './PixelCanvas.css';
import { usePixelStore } from '../stores/pixelStore';

interface PixelCanvasProps {
  pixelSize?: number;
}

export function PixelCanvas({ pixelSize = 10 }: PixelCanvasProps) {
  const { pixels, selectedColor, setPixel, setSelectedColor } =
    usePixelStore();

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [dimensions, setDimensions] = useState({ width: 0, height: 0 });

  console.log('PixelCanvasContent rendering', {
    pixels,
    selectedColor,
    isReady: true,
  });

  // Update canvas size on window resize
  useEffect(() => {
    const updateSize = () => {
      setDimensions({
        width: window.innerWidth,
        height: window.innerHeight,
      });
    };

    updateSize();
    window.addEventListener('resize', updateSize);
    return () => window.removeEventListener('resize', updateSize);
  }, []);

  // Initialize and draw canvas
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !dimensions.width || !dimensions.height) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    // Set canvas size to viewport
    canvas.width = dimensions.width;
    canvas.height = dimensions.height;

    // Draw grid
    drawCanvas(ctx);
  }, [dimensions, pixelSize, pixels]);

  const drawCanvas = (ctx: CanvasRenderingContext2D) => {
    // Clear canvas with white background
    ctx.fillStyle = '#ffffff';
    ctx.fillRect(0, 0, ctx.canvas.width, ctx.canvas.height);

    // Calculate center offset
    const centerX = Math.floor(ctx.canvas.width / 2);
    const centerY = Math.floor(ctx.canvas.height / 2);

    // Draw only placed pixels
    Object.entries(pixels).forEach(([key, pixel]) => {
      const [x, y] = key.split(',').map(Number);
      // Convert from grid coordinates (centered at 0,0) to screen coordinates
      const screenX = centerX + x * pixelSize;
      const screenY = centerY + y * pixelSize;
      ctx.fillStyle = pixel.color;
      ctx.fillRect(screenX, screenY, pixelSize, pixelSize);
    });
  };

  const handleCanvasClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const rect = canvas.getBoundingClientRect();
    // Get canvas coordinates
    const canvasX = e.clientX - rect.left;
    const canvasY = e.clientY - rect.top;

    // Calculate center offset
    const centerX = Math.floor(canvas.width / 2);
    const centerY = Math.floor(canvas.height / 2);

    // Convert to grid coordinates (centered at 0,0)
    const gridX = Math.floor((canvasX - centerX) / pixelSize);
    const gridY = Math.floor((canvasY - centerY) / pixelSize);

    placePixel(gridX, gridY);
  };

  const placePixel = (x: number, y: number) => {
      setPixel(x, y, selectedColor);
  };

  return (
    <div className="pixel-canvas-container">
      <canvas
        ref={canvasRef}
        className="pixel-canvas"
        onClick={handleCanvasClick}
        onMouseDown={handleCanvasClick}
      />

      <div className="chat-bubble">
        <p className="instructions">
          Click to place a pixel, send this website to a friend to draw together!
        </p>

        <div className="color-picker-section">
          <label htmlFor="color-picker" className="color-label">
            Color:
          </label>
          <div className="color-picker-wrapper">
            <input
              type="color"
              id="color-picker"
              value={selectedColor}
              onChange={e => setSelectedColor(e.target.value)}
              className="color-picker"
            />
            <span className="color-value">{selectedColor.toUpperCase()}</span>
          </div>
        </div>
      </div>
    </div>
  );
}
