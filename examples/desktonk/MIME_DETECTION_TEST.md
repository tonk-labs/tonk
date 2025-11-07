# MIME Type Detection Testing Guide

## Overview

The MIME type detection feature automatically detects file types from file extensions and displays
appropriate emoji icons for different file types.

## Icon Mapping

- **Text files** (`.txt`, `.md`, `.csv`): 📄
- **Images** (`.png`, `.jpg`, `.gif`, `.svg`): 🖼️
- **JSON files** (`.json`): 📋
- **PDF files** (`.pdf`): 📕
- **Video files** (`.mp4`, `.avi`, `.mov`): 🎬
- **Audio files** (`.mp3`, `.wav`, `.ogg`): 🎵
- **Other files**: 📦 (default/fallback)

## How to Test

### Option 1: Browser Console Testing

Open the browser console and run the following commands to create test files:

```javascript
// Get the VFS service
const vfs = window.getVFSService?.() || (await import('./lib/vfs-service')).getVFSService();

// Create files with different extensions
await vfs.writeFile('/desktonk/document.txt', { content: { data: 'text content' } }, true);
await vfs.writeFile('/desktonk/data.json', { content: { data: { key: 'value' } } }, true);
await vfs.writeFile('/desktonk/readme.md', { content: { data: '# Title' } }, true);
await vfs.writeFile('/desktonk/image.png', { content: { data: 'fake image data' } }, true);
await vfs.writeFile('/desktonk/video.mp4', { content: { data: 'fake video data' } }, true);
await vfs.writeFile('/desktonk/audio.mp3', { content: { data: 'fake audio data' } }, true);
await vfs.writeFile('/desktonk/document.pdf', { content: { data: 'fake pdf data' } }, true);
await vfs.writeFile('/desktonk/unknown.xyz', { content: { data: 'unknown type' } }, true);
```

### Option 2: Test MIME Detection Directly

You can also test the MIME detection utilities directly:

```javascript
import { getMimeType, getFileIcon, getAppHandler } from './src/features/desktop/utils/mimeResolver';

// Test MIME type detection
console.log('document.txt:', getMimeType('document.txt')); // "text/plain"
console.log('data.json:', getMimeType('data.json')); // "application/json"
console.log('readme.md:', getMimeType('readme.md')); // "text/markdown"
console.log('image.png:', getMimeType('image.png')); // "image/png"
console.log('video.mp4:', getMimeType('video.mp4')); // "video/mp4"

// Test icon selection
console.log('text/plain icon:', getFileIcon('text/plain')); // 📄
console.log('application/json icon:', getFileIcon('application/json')); // 📋
console.log('image/png icon:', getFileIcon('image/png')); // 🖼️
console.log('video/mp4 icon:', getFileIcon('video/mp4')); // 🎬
console.log('audio/mp3 icon:', getFileIcon('audio/mpeg')); // 🎵
console.log('application/pdf icon:', getFileIcon('application/pdf')); // 📕

// Test app handler mapping
console.log('text/plain handler:', getAppHandler('text/plain')); // "text-editor"
console.log('application/json handler:', getAppHandler('application/json')); // "text-editor"
console.log('image/png handler:', getAppHandler('image/png')); // "text-editor" (fallback)
```

## Expected Results

After creating the test files, you should see them on the desktop canvas with different icons:

- `document.txt` → 📄 (text)
- `data.json` → 📋 (json)
- `readme.md` → 📄 (text/markdown)
- `image.png` → 🖼️ (image)
- `video.mp4` → 🎬 (video)
- `audio.mp3` → 🎵 (audio)
- `document.pdf` → 📕 (pdf)
- `unknown.xyz` → 📦 (default)

## Custom Icons

Files can also have custom icons set via the `desktopMeta.icon` property:

```javascript
await vfs.writeFile(
  '/desktonk/custom.txt',
  {
    content: {
      data: 'text with custom icon',
      desktopMeta: {
        icon: '⭐', // Custom star icon
        x: 100,
        y: 100,
      },
    },
  },
  true
);
```

## Implementation Details

### Files Modified

1. **`app/src/features/desktop/utils/mimeResolver.ts`** (new)
   - `getMimeType(fileName)`: Detects MIME type from filename
   - `getFileIcon(mimeType)`: Returns appropriate emoji icon
   - `getAppHandler(mimeType, override?)`: Maps MIME type to app handler
   - `MIME_TO_APP`: Record mapping MIME types to app names

2. **`app/src/features/desktop/utils/fileMetadata.ts`** (updated)
   - Now auto-detects MIME type from filename if not in metadata
   - Falls back to `application/octet-stream` if detection fails

3. **`app/src/features/desktop/shapes/FileIconUtil.tsx`** (updated)
   - Uses `getFileIcon()` to render appropriate icon
   - Supports custom icon override via `shape.props.customIcon`

4. **`app/package.json`** (updated)
   - Added `mime@4.1.0` dependency
   - Added `@types/mime@4.0.0` dev dependency

### Commit

- SHA: `672ea61d159d335d864cab4083748b3d446cc5c6`
- Message: `feat(desktop): add MIME type detection and icons`
