/**
 * Sample Files Creation Script
 *
 * This script provides instructions for creating sample files in the Desktonk desktop environment.
 * Since the VFS service requires a browser context with service worker support, this script
 * cannot be run directly from Node.js/Bun CLI.
 *
 * Instead, the createSampleFiles() function is available in the browser via:
 * 1. Import and use in React components: import { createSampleFiles } from '../utils/sampleFiles'
 * 2. Browser console: Run createSampleFiles() after the app loads
 */

console.log(`
================================================================================
Sample Files Creation Script
================================================================================

This script cannot run in a Node.js/Bun environment because the VFS service
requires a browser context with service worker support.

To create sample files in Desktonk:

METHOD 1 - Browser Console:
----------------------------
1. Start the development server:
   cd packages/app
   bun run dev

2. Open the browser (http://localhost:5173)

3. Open the browser console (F12 or Cmd+Option+I)

4. Run the following command:
   createSampleFiles()

5. The function will create 3 sample files:
   - Welcome.txt (50, 50)
   - README.md (170, 50)
   - notes.json (290, 50)


METHOD 2 - UI Integration:
----------------------------
The createSampleFiles() function is exported from src/utils/sampleFiles.ts
and can be integrated into the app UI as a button or menu item.

Example:
  import { createSampleFiles } from '../utils/sampleFiles';

  <button onClick={() => createSampleFiles()}>
    Create Sample Files
  </button>


SAMPLE FILES INCLUDED:
----------------------
- Welcome.txt: Introduction to Desktonk
- README.md: Comprehensive guide to features
- notes.json: Sample JSON data with notes

All files include desktop metadata for proper positioning.

================================================================================
`);

// Exit with informational message
process.exit(0);
