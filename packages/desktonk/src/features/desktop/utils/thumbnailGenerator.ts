/**
 * Common extensionless text files that should be recognized.
 */
const EXTENSIONLESS_TEXT_FILES = [
  'LICENSE',
  'README',
  'CHANGELOG',
  'CONTRIBUTING',
  'AUTHORS',
  'COPYING',
  'INSTALL',
  'NOTICE',
  'TODO',
  'MAKEFILE',
  'Makefile',
  'Dockerfile',
  'Jenkinsfile',
  'Vagrantfile',
  'Gemfile',
  'Rakefile',
];

/**
 * Gets the icon template path for a given file name.
 * @param fileName - The name of the file
 * @returns Path to the SVG icon template and optional extension label
 */
function getIconTemplatePath(fileName: string): {
  path: string;
  extension: string | null;
} {
  const extension = fileName.split('.').pop()?.toLowerCase() || '';

  // Always use BASE.svg as the template
  // We'll add text dynamically for extensions we want to show

  // Check for extensionless text files - no label
  if (EXTENSIONLESS_TEXT_FILES.includes(fileName)) {
    return { path: '/icon_templates/BASE.svg', extension: null };
  }

  // Extensions we want to show labels for
  const showExtension = [
    'md',
    'txt',
    'json',
    'js',
    'ts',
    'tsx',
    'jsx',
    'css',
    'html',
    'xml',
    'yaml',
    'yml',
    'py',
    'rb',
    'go',
    'rs',
    'java',
    'c',
    'cpp',
    'h',
    'sh',
    'sql',
    'log',
  ];

  if (extension && showExtension.includes(extension)) {
    return {
      path: '/icon_templates/BASE.svg',
      extension: extension.toUpperCase(),
    };
  }

  // Everything else: BASE template with no label
  return { path: '/icon_templates/BASE.svg', extension: null };
}

/**
 * Extracts the clip path from an SVG by finding the path with green fill (#00FF22).
 * @param svgContent - The SVG content as a string
 * @returns The path data string or null if not found
 */
function extractClipPathFromSVG(svgContent: string): string | null {
  try {
    // Strategy 1: Direct green fill
    const directGreenMatch =
      svgContent.match(/<path[^>]*fill="#00FF22"[^>]*d="([^"]+)"/i) ||
      svgContent.match(/<path[^>]*d="([^"]+)"[^>]*fill="#00FF22"/i);
    if (directGreenMatch?.[1]) {
      return directGreenMatch[1];
    }

    // Strategy 2: Gradient fill referencing green
    // Find gradients that contain #00FF22
    const gradientMatch = svgContent.match(
      /<linearGradient[^>]*id="([^"]+)"[^>]*>[\s\S]*?#00FF22[\s\S]*?<\/linearGradient>/i
    );
    if (gradientMatch?.[1]) {
      const gradientId = gradientMatch[1];
      // Find path that uses this gradient
      const pathMatch =
        svgContent.match(
          new RegExp(`<path[^>]*fill="url\\(#${gradientId}\\)"[^>]*d="([^"]+)"`, 'i')
        ) ||
        svgContent.match(
          new RegExp(`<path[^>]*d="([^"]+)"[^>]*fill="url\\(#${gradientId}\\)"`, 'i')
        );
      if (pathMatch?.[1]) {
        return pathMatch[1];
      }
    }

    return null;
  } catch (error) {
    console.error('[thumbnailGenerator] Error extracting clip path:', error);
    return null;
  }
}

/**
 * Generates a thumbnail for an image file.
 * @param file - The image file to generate a thumbnail for
 * @param size - Target size for the thumbnail (default 256x256)
 * @returns Base64 data URL of the thumbnail, or null if generation fails
 */
export async function generateImageThumbnail(
  file: File,
  size: number = 512 // Double resolution for better quality
): Promise<string | null> {
  return new Promise((resolve) => {
    // Only process image files
    if (!file.type.startsWith('image/')) {
      resolve(null);
      return;
    }

    const reader = new FileReader();

    reader.onload = (e) => {
      const img = new Image();

      img.onload = () => {
        try {
          // Create canvas for thumbnail
          const canvas = document.createElement('canvas');
          const ctx = canvas.getContext('2d');

          if (!ctx) {
            console.error('[thumbnailGenerator] Failed to get canvas context');
            resolve(null);
            return;
          }

          // Calculate dimensions maintaining aspect ratio
          let width = img.width;
          let height = img.height;

          if (width > height) {
            if (width > size) {
              height = (height * size) / width;
              width = size;
            }
          } else {
            if (height > size) {
              width = (width * size) / height;
              height = size;
            }
          }

          // Set canvas size
          canvas.width = size;
          canvas.height = size;

          // Fill with transparent background
          ctx.clearRect(0, 0, size, size);

          // Center the image on canvas
          const x = (size - width) / 2;
          const y = (size - height) / 2;

          // Draw image
          ctx.drawImage(img, x, y, width, height);

          // Convert to base64 data URL (using PNG to preserve transparency)
          const thumbnail = canvas.toDataURL('image/png');
          resolve(thumbnail);
        } catch (error) {
          console.error('[thumbnailGenerator] Error generating thumbnail:', error);
          resolve(null);
        }
      };

      img.onerror = () => {
        console.error('[thumbnailGenerator] Failed to load image');
        resolve(null);
      };

      img.src = e.target?.result as string;
    };

    reader.onerror = () => {
      console.error('[thumbnailGenerator] Failed to read file');
      resolve(null);
    };

    reader.readAsDataURL(file);
  });
}

/**
 * Core thumbnail generation logic from text content.
 * @param text - The text content to generate a thumbnail from
 * @param fileName - The file name (used for template selection)
 * @param size - Target size for the thumbnail (default 512x512)
 * @returns Base64 data URL of the thumbnail, or null if generation fails
 */
async function generateTextThumbnailCore(
  text: string,
  fileName: string,
  size: number = 512
): Promise<string | null> {
  return new Promise((resolve) => {
    try {
      // Calculate dimensions of the clip area in SVG coordinates
      const svgContentX = 18;
      const svgContentY = 8;
      const svgContentWidth = 58;
      const svgContentHeight = 82;

      // Scale factors from SVG viewBox to canvas size
      const scaleX = size / 94;
      const scaleY = size / 104;

      // Calculate the actual pixel dimensions of the rectangular clip area
      const textCanvasWidth = Math.round(svgContentWidth * scaleX);
      const textCanvasHeight = Math.round(svgContentHeight * scaleY);

      // Step 1: Create rectangular text content canvas (matches clip area aspect ratio)
      const textCanvas = document.createElement('canvas');
      const textCtx = textCanvas.getContext('2d');

      if (!textCtx) {
        console.error('[thumbnailGenerator] Failed to get canvas context');
        resolve(null);
        return;
      }

      textCanvas.width = textCanvasWidth;
      textCanvas.height = textCanvasHeight;

      // Fill with white background
      textCtx.fillStyle = '#ffffff';
      textCtx.fillRect(0, 0, textCanvasWidth, textCanvasHeight);

      // Configure text style (scaled for 2x resolution)
      textCtx.fillStyle = '#000000';
      textCtx.font = "16px 'Monaco', 'Menlo', 'Courier New', monospace";
      textCtx.textBaseline = 'top';

      // Process text: split by lines and take first 18 lines
      const lines = text.split('\n').slice(0, 18);
      const lineHeight = 24; // 2x
      const padding = 24; // 2x - Extra padding for gutter/bleed
      const padding_top = 28;

      // Draw each line
      lines.forEach((line, index) => {
        // Convert tabs to spaces and truncate to 30 characters
        const processedLine = line.replace(/\t/g, '  ').substring(0, 30);
        const y = padding_top + index * lineHeight;
        textCtx.fillText(processedLine, padding, y);
      });

      // Step 2: Load and process SVG template (async)
      (async () => {
        const templateInfo = getIconTemplatePath(fileName);
        console.log(
          '[thumbnailGenerator] Using template:',
          templateInfo.path,
          'for file:',
          fileName,
          'extension:',
          templateInfo.extension
        );

        try {
          // Fetch SVG template
          const response = await fetch(templateInfo.path);
          if (!response.ok) {
            throw new Error(`Failed to load template: ${response.status}`);
          }

          let svgContent = await response.text();
          console.log('[thumbnailGenerator] Successfully loaded SVG template');

          // Add dynamic text element for the extension if we have one
          if (templateInfo.extension) {
            const textElement = `<text x="47" y="80" font-family="system-ui, -apple-system, sans-serif" font-size="9" font-weight="600" fill="black" text-anchor="middle">${templateInfo.extension}</text>`;
            svgContent = svgContent.replace('</svg>', `${textElement}</svg>`);
          }

          console.log('[thumbnailGenerator] SVG content length:', svgContent.length);
          console.log('[thumbnailGenerator] Contains green?', svgContent.includes('#00FF22'));

          // Extract clip path
          const clipPathData = extractClipPathFromSVG(svgContent);
          console.log('[thumbnailGenerator] Clip path extracted:', clipPathData ? 'YES' : 'NO');

          if (!clipPathData) {
            console.warn('[thumbnailGenerator] No clip path found, using plain thumbnail');
            const thumbnail = textCanvas.toDataURL('image/png');
            resolve(thumbnail);
            return;
          }

          console.log('[thumbnailGenerator] Extracted clip path, length:', clipPathData.length);

          // Step 3: Create final composite canvas
          const finalCanvas = document.createElement('canvas');
          const finalCtx = finalCanvas.getContext('2d');

          if (!finalCtx) {
            console.error('[thumbnailGenerator] Failed to get final canvas context');
            resolve(null);
            return;
          }

          finalCanvas.width = size;
          finalCanvas.height = size;

          // Clear to transparent background
          finalCtx.clearRect(0, 0, size, size);

          // Step 4: Apply clipping and draw text thumbnail
          // The text canvas is already rectangular and matches the clip area dimensions

          const scaledX = svgContentX * scaleX;
          const scaledY = svgContentY * scaleY;

          finalCtx.save();

          // Create clip path from SVG path data in pixel coordinates
          const clipPath = new Path2D(clipPathData);

          // Create a transformed path that's scaled to pixel coordinates
          const matrix = new DOMMatrix();
          matrix.a = scaleX;
          matrix.d = scaleY;
          const scaledClipPath = new Path2D();
          scaledClipPath.addPath(clipPath, matrix);

          // Apply clipping in pixel space
          finalCtx.clip(scaledClipPath);

          // Draw rectangular text canvas - draw at exact clip area dimensions
          finalCtx.drawImage(textCanvas, scaledX, scaledY, textCanvasWidth, textCanvasHeight);

          finalCtx.restore();

          // Step 5: Process alpha and draw SVG template on top

          // First, find gradient ID for green if it exists
          const gradientMatch = svgContent.match(
            /<linearGradient[^>]*id="([^"]+)"[^>]*>[\s\S]*?#00FF22[\s\S]*?<\/linearGradient>/i
          );
          const greenGradientId = gradientMatch ? gradientMatch[1] : null;

          // Create SVG WITH green for alpha detection
          const svgBlobWithGreen = new Blob([svgContent], {
            type: 'image/svg+xml',
          });
          const svgUrlWithGreen = URL.createObjectURL(svgBlobWithGreen);

          const svgImgWithGreen = new Image();

          svgImgWithGreen.onload = () => {
            // Render SVG with green to detect alpha
            const svgCanvas = document.createElement('canvas');
            const svgCtx = svgCanvas.getContext('2d');

            if (!svgCtx) {
              console.error('[thumbnailGenerator] Failed to get SVG canvas context');
              URL.revokeObjectURL(svgUrlWithGreen);
              resolve(textCanvas.toDataURL('image/png'));
              return;
            }

            svgCanvas.width = size;
            svgCanvas.height = size;
            svgCtx.drawImage(svgImgWithGreen, 0, 0, size, size);

            // Get image data from both canvases
            const svgData = svgCtx.getImageData(0, 0, size, size);
            const finalData = finalCtx.getImageData(0, 0, size, size);

            // Process pixels: blend semi-transparent green toward white
            for (let i = 0; i < svgData.data.length; i += 4) {
              const r = svgData.data[i];
              const g = svgData.data[i + 1];
              const b = svgData.data[i + 2];
              const a = svgData.data[i + 3];

              // Check if pixel is greenish (#00FF22 with tolerance)
              const isGreen = r < 50 && g > 200 && b < 100;

              if (isGreen && a > 0 && a < 255) {
                // Semi-transparent green - blend toward white
                const whiteness = (255 - a) / 255; // How much to blend toward white

                // Get current pixel from final canvas (clipped text)
                const currentR = finalData.data[i];
                const currentG = finalData.data[i + 1];
                const currentB = finalData.data[i + 2];

                // Blend with white based on green's transparency
                finalData.data[i] = currentR + (255 - currentR) * whiteness;
                finalData.data[i + 1] = currentG + (255 - currentG) * whiteness;
                finalData.data[i + 2] = currentB + (255 - currentB) * whiteness;
                finalData.data[i + 3] = 255; // Fully opaque
              }
            }

            // Put processed data back
            finalCtx.putImageData(finalData, 0, 0);

            // Clean up first SVG
            URL.revokeObjectURL(svgUrlWithGreen);

            // Now render SVG WITHOUT green for final overlay
            let svgWithoutGreen = svgContent.replace(/fill="#00FF22"/gi, 'fill="none"');
            if (greenGradientId) {
              svgWithoutGreen = svgWithoutGreen.replace(
                new RegExp(`fill="url\\(#${greenGradientId}\\)"`, 'gi'),
                'fill="none"'
              );
            }

            const svgBlobWithoutGreen = new Blob([svgWithoutGreen], {
              type: 'image/svg+xml',
            });
            const svgUrlWithoutGreen = URL.createObjectURL(svgBlobWithoutGreen);

            const svgImgWithoutGreen = new Image();

            svgImgWithoutGreen.onload = () => {
              // Draw final SVG overlay (borders, shadows, etc.)
              finalCtx.drawImage(svgImgWithoutGreen, 0, 0, size, size);

              // Clean up
              URL.revokeObjectURL(svgUrlWithoutGreen);

              // Convert to base64 data URL (use PNG to preserve transparency)
              const thumbnail = finalCanvas.toDataURL('image/png');
              resolve(thumbnail);
            };

            svgImgWithoutGreen.onerror = () => {
              console.error('[thumbnailGenerator] Failed to load SVG without green');
              URL.revokeObjectURL(svgUrlWithoutGreen);
              const thumbnail = finalCanvas.toDataURL('image/png');
              resolve(thumbnail);
            };

            svgImgWithoutGreen.src = svgUrlWithoutGreen;
          };

          svgImgWithGreen.onerror = () => {
            console.error('[thumbnailGenerator] Failed to load SVG with green');
            URL.revokeObjectURL(svgUrlWithGreen);
            // Fallback to plain thumbnail
            const thumbnail = textCanvas.toDataURL('image/png');
            resolve(thumbnail);
          };

          svgImgWithGreen.src = svgUrlWithGreen;
        } catch (error) {
          console.error('[thumbnailGenerator] Error loading icon template:', error);
          // Fallback to plain text thumbnail without template
          const thumbnail = textCanvas.toDataURL('image/png');
          resolve(thumbnail);
        }
      })();
    } catch (error) {
      console.error('[thumbnailGenerator] Error generating text thumbnail:', error);
      resolve(null);
    }
  });
}

/**
 * Generates a thumbnail from text content and file name.
 * @param text - The text content to generate a thumbnail from
 * @param fileName - The file name (used for template selection)
 * @param size - Target size for the thumbnail (default 512x512)
 * @returns Base64 data URL of the thumbnail, or null if generation fails
 */
export async function generateTextThumbnailFromContent(
  text: string,
  fileName: string,
  size: number = 512
): Promise<string | null> {
  return generateTextThumbnailCore(text, fileName, size);
}

/**
 * Generates a thumbnail for a text file with icon template overlay.
 * @param file - The text file to generate a thumbnail for
 * @param size - Target size for the thumbnail (default 256x256)
 * @returns Base64 data URL of the thumbnail, or null if generation fails
 */
export async function generateTextThumbnail(
  file: File,
  size: number = 512 // Double resolution for better quality
): Promise<string | null> {
  return new Promise((resolve) => {
    const reader = new FileReader();

    reader.onload = async (e) => {
      const text = e.target?.result as string;
      const thumbnail = await generateTextThumbnailCore(text, file.name, size);
      resolve(thumbnail);
    };

    reader.onerror = () => {
      console.error('[thumbnailGenerator] Failed to read text file');
      resolve(null);
    };

    // Read file as text (limit to first 5KB for performance)
    const blob = file.slice(0, 5120); // 5KB
    reader.readAsText(blob);
  });
}
