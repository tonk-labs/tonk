/**
 * Generates a thumbnail for an image file.
 * @param file - The image file to generate a thumbnail for
 * @param size - Target size for the thumbnail (default 256x256)
 * @returns Base64 data URL of the thumbnail, or null if generation fails
 */
export async function generateImageThumbnail(
  file: File,
  size: number = 256
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

          // Convert to base64 data URL (using JPEG for smaller size)
          const thumbnail = canvas.toDataURL('image/jpeg', 0.85);
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
