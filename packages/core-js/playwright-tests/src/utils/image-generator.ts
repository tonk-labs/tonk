import * as fs from 'fs/promises';
import * as path from 'path';
import type { ImageSpec, TestImage } from '../test-ui/types';

/**
 * Synthetic image generator for stress testing
 * Creates realistic test data without requiring DOM APIs
 */
export class ImageGenerator {
  private cacheDir = '.image-cache';
  /**
   * Generate synthetic image data with specified characteristics
   */
  async generateImage(spec: ImageSpec): Promise<Uint8Array> {
    const filename = `${spec.width}x${spec.height}-${spec.sizeInMB}MB-${spec.format}.bin`;
    const cachePath = path.join(this.cacheDir, filename);

    try {
      const data = await fs.readFile(cachePath);
      return new Uint8Array(data);
    } catch {
      const targetSizeBytes = Math.floor(spec.sizeInMB * 1024 * 1024);
      const data = this.generateSyntheticImageData(targetSizeBytes, spec);

      await fs.mkdir(this.cacheDir, { recursive: true });
      await fs.writeFile(cachePath, data);

      return data;
    }
  }

  /**
   * Generate synthetic image data that resembles real image file structure
   */
  private generateSyntheticImageData(
    sizeBytes: number,
    spec: ImageSpec
  ): Uint8Array {
    const data = new Uint8Array(sizeBytes);

    // Add format-specific headers for realism
    if (spec.format === 'jpeg') {
      // JPEG SOI marker
      data[0] = 0xff;
      data[1] = 0xd8;
      data[2] = 0xff;
      data[3] = 0xe0; // JFIF marker
      // Add JFIF header
      data[4] = 0x00;
      data[5] = 0x10; // Length
      data[6] = 0x4a; // 'J'
      data[7] = 0x46; // 'F'
      data[8] = 0x49; // 'I'
      data[9] = 0x46; // 'F'
      data[10] = 0x00; // Null terminator
    } else if (spec.format === 'png') {
      // PNG signature
      data[0] = 0x89;
      data[1] = 0x50; // 'P'
      data[2] = 0x4e; // 'N'
      data[3] = 0x47; // 'G'
      data[4] = 0x0d;
      data[5] = 0x0a;
      data[6] = 0x1a;
      data[7] = 0x0a;
    } else if (spec.format === 'webp') {
      // WebP signature
      data[0] = 0x52; // 'R'
      data[1] = 0x49; // 'I'
      data[2] = 0x46; // 'F'
      data[3] = 0x46; // 'F'
      // Size will be filled later
      data[8] = 0x57; // 'W'
      data[9] = 0x45; // 'E'
      data[10] = 0x42; // 'B'
      data[11] = 0x50; // 'P'
    }

    // Fill the rest with pseudo-random data that varies by image characteristics
    const seed = spec.width * spec.height + spec.sizeInMB * 1000;
    let randomSeed = seed;

    const headerSize =
      spec.format === 'png' ? 8 : spec.format === 'webp' ? 12 : 11;

    for (let i = headerSize; i < sizeBytes; i++) {
      // Simple linear congruential generator for consistent but varied data
      randomSeed = (randomSeed * 1664525 + 1013904223) % 0x100000000;

      // Create patterns that simulate image compression
      if (i % 64 === 0) {
        // Simulate DCT block boundaries in JPEG or similar structures
        data[i] = (randomSeed % 128) + 64;
      } else if (i % 8 === 0) {
        // Simulate quantization patterns or PNG filtering
        data[i] = randomSeed % 256;
      } else {
        // Regular compressed data with some structure
        const position = i / sizeBytes;
        const pattern = Math.sin(position * Math.PI * 4) * 32 + 128;
        data[i] = ((randomSeed + i) % 128) + Math.floor(pattern);
      }
    }

    return data;
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
        console.log(`Generated ${i}/${count} synthetic images...`);
      }
    }

    return images;
  }

  /**
   * Generate or load cached test images for a specific test
   */
  async generateBatchForTest(
    testName: string,
    count: number,
    sizeRangeMB: [number, number] = [1, 10],
    formats: ('jpeg' | 'png' | 'webp')[] = ['jpeg', 'png']
  ): Promise<TestImage[]> {
    const testCacheDir = path.join(this.cacheDir, testName);

    try {
      await fs.access(testCacheDir);
      const cachedImages = await this.loadCachedImages(testCacheDir, count);
      if (cachedImages.length === count) {
        console.log(`Loaded ${count} cached images for test: ${testName}`);
        return cachedImages;
      }
    } catch {
      console.log(
        `No cache found for test: ${testName}, generating ${count} images...`
      );
    }

    const images = await this.generateBatch(count, sizeRangeMB, formats);
    await this.saveBatchToCache(testCacheDir, images);
    console.log(`Generated and cached ${count} images for test: ${testName}`);
    return images;
  }

  /**
   * Load cached images from a test directory
   */
  private async loadCachedImages(
    cacheDir: string,
    expectedCount: number
  ): Promise<TestImage[]> {
    const files = await fs.readdir(cacheDir);
    const imageFiles = files
      .filter(f => f.endsWith('.bin'))
      .slice(0, expectedCount);

    if (imageFiles.length < expectedCount) {
      throw new Error(
        `Not enough cached images: found ${imageFiles.length}, need ${expectedCount}`
      );
    }

    const images: TestImage[] = [];
    for (const file of imageFiles) {
      const filePath = path.join(cacheDir, file);
      const data = await fs.readFile(filePath);
      const metadata = this.parseImageMetadataFromFilename(file);

      images.push({
        name: file.replace('.bin', ''),
        data: new Uint8Array(data),
        size: data.byteLength,
        metadata,
      });
    }

    return images;
  }

  /**
   * Save a batch of images to cache directory
   */
  private async saveBatchToCache(
    cacheDir: string,
    images: TestImage[]
  ): Promise<void> {
    await fs.mkdir(cacheDir, { recursive: true });

    for (let i = 0; i < images.length; i++) {
      const image = images[i];
      const filename = `${i}_${image.metadata.width}x${image.metadata.height}_${(image.size / (1024 * 1024)).toFixed(2)}MB_${image.metadata.format}.bin`;
      const filePath = path.join(cacheDir, filename);
      await fs.writeFile(filePath, image.data);
    }
  }

  /**
   * Parse image metadata from cached filename
   */
  private parseImageMetadataFromFilename(filename: string) {
    const match = filename.match(/(\d+)_(\d+)x(\d+)_[\d.]+MB_(\w+)\.bin/);
    if (match) {
      return {
        width: parseInt(match[2]),
        height: parseInt(match[3]),
        format: match[4] as 'jpeg' | 'png' | 'webp',
      };
    }
    return { width: 1920, height: 1080, format: 'jpeg' as const };
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
      { width: 1920, height: 1080 }, // HD landscape
      { width: 1080, height: 1920 }, // HD portrait
    ];

    return dimensions[Math.floor(Math.random() * dimensions.length)];
  }

  /**
   * Generate test data for a specific file size (for large file tests)
   */
  async generateLargeFile(
    sizeMB: number,
    format: 'jpeg' | 'png' | 'webp' = 'jpeg'
  ): Promise<TestImage> {
    const dimensions = { width: 4032, height: 3024 }; // Large iPhone photo size

    const spec: ImageSpec = {
      width: dimensions.width,
      height: dimensions.height,
      sizeInMB: sizeMB,
      format,
      quality: format === 'jpeg' ? 0.9 : undefined,
    };

    const data = await this.generateImage(spec);

    return {
      name: `large-file-${sizeMB}MB.${format}`,
      data,
      size: data.byteLength,
      metadata: {
        width: dimensions.width,
        height: dimensions.height,
        format,
      },
    };
  }

  /**
   * Create a small test file for quick operations
   */
  async generateSmallFile(sizeKB: number = 100): Promise<TestImage> {
    const sizeMB = sizeKB / 1024;
    const dimensions = { width: 640, height: 480 }; // Small dimensions

    const spec: ImageSpec = {
      width: dimensions.width,
      height: dimensions.height,
      sizeInMB: sizeMB,
      format: 'jpeg',
      quality: 0.7,
    };

    const data = await this.generateImage(spec);

    return {
      name: `small-file-${sizeKB}KB.jpeg`,
      data,
      size: data.byteLength,
      metadata: {
        width: dimensions.width,
        height: dimensions.height,
        format: 'jpeg',
      },
    };
  }
}

// Export singleton instance for convenience
export const imageGenerator = new ImageGenerator();
