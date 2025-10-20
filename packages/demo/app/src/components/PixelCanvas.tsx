import { useRef, useEffect } from 'react';
import './PixelCanvas.css';
import { useVFS } from '../hooks/useVFS';
import { usePixelStore } from '../stores/pixelStore';

interface PixelCanvasProps {
  width?: number;
  height?: number;
  pixelSize?: number;
}

export function PixelCanvas({
  width = 32,
  height = 48,
  pixelSize = 10,
}: PixelCanvasProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  // const { connectionState } = useVFS();

  // console.log('PixelCanvas rendering', { connectionState });

  // if (connectionState !== 'connected') {
  //   return (
  //     <div className="pixel-canvas-container">
  //       <div className="toolbar">
  //         <h2 className="canvas-title">Collaborative Pixel Editor</h2>
  //         <p className="instructions">Loading...</p>
  //       </div>
  //     </div>
  //   );
  // }

  return (
    <PixelCanvasContent
      width={width}
      height={height}
      pixelSize={pixelSize}
      canvasRef={canvasRef}
    />
  );
}

function PixelCanvasContent({
  width = 32,
  height = 48,
  pixelSize = 10,
  canvasRef,
}: PixelCanvasProps & {
  canvasRef: React.RefObject<HTMLCanvasElement | null>;
}) {
  const { pixels, selectedColor, setPixel, removePixel, setSelectedColor } =
    usePixelStore();

  console.log('PixelCanvasContent rendering', {
    pixels,
    selectedColor,
    isReady: true,
  });

  // Initialize canvas
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    // Set canvas size
    canvas.width = width * pixelSize;
    canvas.height = height * pixelSize;

    // Draw grid
    drawCanvas(ctx);
  }, [width, height, pixelSize, pixels]);

  const drawCanvas = (ctx: CanvasRenderingContext2D) => {
    // Clear canvas with white background
    ctx.fillStyle = '#ffffff';
    ctx.fillRect(0, 0, ctx.canvas.width, ctx.canvas.height);

    // Draw only placed pixels
    Object.entries(pixels).forEach(([key, pixel]) => {
      const [x, y] = key.split(',').map(Number);
      ctx.fillStyle = pixel.color;
      ctx.fillRect(x * pixelSize, y * pixelSize, pixelSize, pixelSize);
    });
  };

  const handleCanvasClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const rect = canvas.getBoundingClientRect();
    const scaleX = canvas.width / rect.width;
    const scaleY = canvas.height / rect.height;
    const x = Math.floor(((e.clientX - rect.left) * scaleX) / pixelSize);
    const y = Math.floor(((e.clientY - rect.top) * scaleY) / pixelSize);

    if (x >= 0 && x < width && y >= 0 && y < height) {
      placePixel(x, y);
    }
  };

  const placePixel = (x: number, y: number) => {
    const key = `${x},${y}`;
    const existingPixel = pixels[key];

    // Toggle off if clicking the same color, otherwise place new color
    if (existingPixel && existingPixel.color === selectedColor) {
      removePixel(x, y);
    } else {
      setPixel(x, y, selectedColor);
    }
  };

  return (
    <div className="pixel-canvas-container">
      <div className="toolbar">
        <h2 className="canvas-title">Collaborative Pixel Editor</h2>
        <p className="instructions">
          Click to place a pixel, click to undo a pixel. Send this demo to a
          friend and click pixels together!
        </p>

        <div className="toolbar-controls">
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

      <div className="canvas-wrapper">
        <canvas
          ref={canvasRef}
          className="pixel-canvas"
          onClick={handleCanvasClick}
        />
      </div>
    </div>
  );
}
