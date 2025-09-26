import { ImageSpec, TestImage } from '../test-ui/types';

/**
 * Synthetic image generator for stress testing
 * Creates realistic images similar to iPhone photos
 */
export class ImageGenerator {
  /**
   * Generate a synthetic image with specified characteristics
   */
  async generateImage(spec: ImageSpec): Promise<Uint8Array> {
    // Create canvas with specified dimensions
    const canvas = document.createElement('canvas');
    canvas.width = spec.width;
    canvas.height = spec.height;
    const ctx = canvas.getContext('2d');

    if (!ctx) {
      throw new Error('Failed to get 2D context');
    }

    // Generate a photo-like pattern
    this.generatePhotoPattern(ctx, spec);

    // Convert to blob with target size
    const blob = await this.canvasToBlob(canvas, spec.format, spec.sizeInMB);

    // Convert blob to Uint8Array
    return new Uint8Array(await blob.arrayBuffer());
  }

  /**
   * Generate multiple test images with varying characteristics
   */
  async generateBatch(
    count: number,
    sizeRangeMB: [number, number] = [1, 10],
    formats: ('jpeg' | 'png' | 'webp')[] = ['jpeg', 'png']
  ): Promise<TestImage[]> {
    const images: TestImage[] = [];

    for (let i = 0; i < count; i++) {
      // Vary dimensions slightly (iPhone photo dimensions)
      const dimensions = this.getRandomDimensions();

      // Random size within range
      const sizeInMB =
        Math.random() * (sizeRangeMB[1] - sizeRangeMB[0]) + sizeRangeMB[0];

      // Random format
      const format = formats[Math.floor(Math.random() * formats.length)];

      const spec: ImageSpec = {
        width: dimensions.width,
        height: dimensions.height,
        sizeInMB,
        format,
        quality: format === 'jpeg' ? 0.8 : undefined,
      };

      const data = await this.generateImage(spec);

      images.push({
        name: `test-image-${i}.${format}`,
        data,
        size: data.byteLength,
        metadata: {
          width: dimensions.width,
          height: dimensions.height,
          format,
        },
      });

      // Log progress for large batches
      if (count > 100 && i % 100 === 0) {
        console.log(`Generated ${i}/${count} images...`);
      }
    }

    return images;
  }

  /**
   * Generate a photo-like pattern on the canvas
   */
  private generatePhotoPattern(
    ctx: CanvasRenderingContext2D,
    spec: ImageSpec
  ): void {
    const { width, height } = spec;

    // Create a gradient background (sky-like)
    const gradient = ctx.createLinearGradient(0, 0, 0, height);
    gradient.addColorStop(0, `hsl(${200 + Math.random() * 40}, 70%, 60%)`);
    gradient.addColorStop(0.4, `hsl(${190 + Math.random() * 30}, 60%, 70%)`);
    gradient.addColorStop(1, `hsl(${180 + Math.random() * 20}, 50%, 80%)`);

    ctx.fillStyle = gradient;
    ctx.fillRect(0, 0, width, height);

    // Add some "landscape" elements
    this.drawLandscape(ctx, width, height);

    // Add noise for realism
    this.addNoise(ctx, width, height);

    // Add some geometric shapes to vary content
    this.drawRandomShapes(ctx, width, height);

    // Add timestamp-like text
    this.drawTimestamp(ctx, width, height);
  }

  /**
   * Draw landscape elements
   */
  private drawLandscape(
    ctx: CanvasRenderingContext2D,
    width: number,
    height: number
  ): void {
    // Draw mountains
    ctx.fillStyle = `hsla(${220 + Math.random() * 20}, 30%, 40%, 0.8)`;
    ctx.beginPath();
    ctx.moveTo(0, height * 0.6);

    // Create random mountain peaks
    for (let x = 0; x <= width; x += width / 8) {
      const y = height * 0.6 - Math.random() * height * 0.3;
      ctx.lineTo(x, y);
    }

    ctx.lineTo(width, height);
    ctx.lineTo(0, height);
    ctx.closePath();
    ctx.fill();

    // Draw ground
    ctx.fillStyle = `hsl(${90 + Math.random() * 30}, 40%, 35%)`;
    ctx.fillRect(0, height * 0.8, width, height * 0.2);
  }

  /**
   * Add noise to make the image more realistic
   */
  private addNoise(
    ctx: CanvasRenderingContext2D,
    width: number,
    height: number
  ): void {
    const imageData = ctx.getImageData(0, 0, width, height);
    const data = imageData.data;

    // Add subtle noise
    for (let i = 0; i < data.length; i += 4) {
      const noise = (Math.random() - 0.5) * 20;
      data[i] = Math.max(0, Math.min(255, data[i] + noise)); // R
      data[i + 1] = Math.max(0, Math.min(255, data[i + 1] + noise)); // G
      data[i + 2] = Math.max(0, Math.min(255, data[i + 2] + noise)); // B
    }

    ctx.putImageData(imageData, 0, 0);
  }

  /**
   * Draw random shapes for content variety
   */
  private drawRandomShapes(
    ctx: CanvasRenderingContext2D,
    width: number,
    height: number
  ): void {
    const shapeCount = 3 + Math.floor(Math.random() * 5);

    for (let i = 0; i < shapeCount; i++) {
      ctx.fillStyle = `hsla(${Math.random() * 360}, 50%, 50%, 0.3)`;

      const x = Math.random() * width;
      const y = Math.random() * height;
      const size = 20 + Math.random() * 100;

      if (Math.random() > 0.5) {
        // Draw circle
        ctx.beginPath();
        ctx.arc(x, y, size / 2, 0, Math.PI * 2);
        ctx.fill();
      } else {
        // Draw rectangle
        ctx.fillRect(x - size / 2, y - size / 2, size, size);
      }
    }
  }

  /**
   * Draw timestamp text
   */
  private drawTimestamp(
    ctx: CanvasRenderingContext2D,
    width: number,
    height: number
  ): void {
    const date = new Date();
    const timestamp =
      date.toISOString().split('T')[0] +
      ' ' +
      date.toTimeString().split(' ')[0];

    ctx.fillStyle = 'rgba(255, 255, 255, 0.8)';
    ctx.font = `${Math.floor(width / 40)}px monospace`;
    ctx.fillText(timestamp, 20, height - 20);
  }

  /**
   * Convert canvas to blob with target size
   */
  private async canvasToBlob(
    canvas: HTMLCanvasElement,
    format: 'jpeg' | 'png' | 'webp',
    targetSizeMB: number
  ): Promise<Blob> {
    const mimeType = `image/${format}`;
    let quality = format === 'jpeg' ? 0.9 : 1;
    let blob: Blob | null = null;

    // Binary search for the right quality to achieve target size
    let minQuality = 0.1;
    let maxQuality = 1;
    let attempts = 0;
    const maxAttempts = 10;
    const tolerance = 0.1; // 10% tolerance

    while (attempts < maxAttempts) {
      blob = await new Promise<Blob | null>(resolve => {
        canvas.toBlob(resolve, mimeType, quality);
      });

      if (!blob) {
        throw new Error('Failed to create blob');
      }

      const sizeMB = blob.size / (1024 * 1024);
      const diff = sizeMB - targetSizeMB;
      const relativeDiff = Math.abs(diff) / targetSizeMB;

      // If within tolerance, we're done
      if (relativeDiff < tolerance) {
        break;
      }

      // Adjust quality for next iteration
      if (sizeMB > targetSizeMB) {
        maxQuality = quality;
        quality = (minQuality + quality) / 2;
      } else {
        minQuality = quality;
        quality = (quality + maxQuality) / 2;
      }

      attempts++;
    }

    if (!blob) {
      throw new Error('Failed to generate blob');
    }

    return blob;
  }

  /**
   * Get random dimensions similar to iPhone photos
   */
  private getRandomDimensions(): { width: number; height: number } {
    const dimensions = [
      { width: 3024, height: 4032 }, // iPhone 11/12/13 portrait
      { width: 4032, height: 3024 }, // iPhone 11/12/13 landscape
      { width: 3088, height: 2316 }, // iPhone X landscape
      { width: 2316, height: 3088 }, // iPhone X portrait
      { width: 2448, height: 3264 }, // iPhone 8 portrait
      { width: 3264, height: 2448 }, // iPhone 8 landscape
    ];

    return dimensions[Math.floor(Math.random() * dimensions.length)];
  }
}

// Export singleton instance for convenience
export const imageGenerator = new ImageGenerator();
