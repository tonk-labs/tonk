/**
 * Cross-Origin Drag & Drop Handler
 *
 * Enables tonk.xyz parent page to send .tonk files to ensemble.tonk.xyz iframe
 * via postMessage API. Provides same visual feedback as native drag-drop.
 *
 * This module is loaded by index.html and calls setupCrossOriginDragDrop()
 * during initialization.
 */

/**
 * Allowed origins for postMessage communication
 * Only these domains can send files to the bootloader
 */
const ALLOWED_ORIGINS = [
  'https://tonk.xyz',
  'https://www.tonk.xyz',
  'http://localhost:3000',
  'http://localhost:5173',
  'http://localhost:5174',
];

/**
 * Maximum file size in bytes (50MB)
 * Prevents DOS attacks via huge files
 */
const MAX_FILE_SIZE = 50 * 1024 * 1024;

/**
 * Set up listener for cross-origin drag-and-drop from parent frame
 * Allows tonk.xyz to send files via postMessage
 *
 * This function should be called during app initialization,
 * after the DOM is ready but before user interaction.
 */
export function setupCrossOriginDragDrop() {
  window.addEventListener('message', handleMessage);
  console.log('[Cross-Origin D&D] Listener initialized');
}

/**
 * Main message handler
 * Routes messages to appropriate handlers based on type
 *
 * @param {MessageEvent} event - The message event from postMessage
 */
async function handleMessage(event) {
  // Validate origin - only accept from allowed domains
  if (!ALLOWED_ORIGINS.includes(event.origin)) {
    console.warn('[Cross-Origin D&D] Rejected message from unauthorized origin:', event.origin);
    return;
  }

  const { type, fileName, fileData } = event.data;

  // Only handle tonk: prefixed messages
  if (!type || !type.startsWith('tonk:')) {
    return;
  }

  console.log('[Cross-Origin D&D] Received message:', type);

  // Route to appropriate handler
  switch (type) {
    case 'tonk:dragEnter':
      handleCrossOriginDragEnter();
      break;

    case 'tonk:dragLeave':
      handleCrossOriginDragLeave();
      break;

    case 'tonk:drop':
      await handleCrossOriginDrop(fileName, fileData);
      break;

    default:
      console.warn('[Cross-Origin D&D] Unknown message type:', type);
      break;
  }
}

/**
 * Visual feedback when parent frame indicates drag enter
 * Adds the 'drag-over' class to highlight the drop zone
 */
function handleCrossOriginDragEnter() {
  const activeMenu = getActiveBootMenu();
  if (activeMenu) {
    activeMenu.classList.add('drag-over');
    console.log('[Cross-Origin D&D] Drag enter - highlighting drop zone');
  }
}

/**
 * Remove visual feedback when drag leaves iframe area
 * Removes the 'drag-over' class
 */
function handleCrossOriginDragLeave() {
  const activeMenu = getActiveBootMenu();
  if (activeMenu) {
    activeMenu.classList.remove('drag-over');
    console.log('[Cross-Origin D&D] Drag leave - removing highlight');
  }
}

/**
 * Process file dropped from parent frame
 * Validates and processes the file using existing bootloader logic
 *
 * @param {string} fileName - Name of the .tonk file
 * @param {ArrayBuffer} fileData - File contents as ArrayBuffer
 */
async function handleCrossOriginDrop(fileName, fileData) {
  console.log('[Cross-Origin D&D] Processing drop:', fileName);

  // Remove drag-over styling immediately
  handleCrossOriginDragLeave();

  // Validate file name
  if (!fileName || typeof fileName !== 'string') {
    showError('Invalid file name received');
    console.error('[Cross-Origin D&D] Missing or invalid fileName');
    return;
  }

  // Validate file type
  if (!fileName.toLowerCase().endsWith('.tonk')) {
    showError('Please drop only .tonk files');
    console.error('[Cross-Origin D&D] Invalid file type:', fileName);
    return;
  }

  // Validate file data exists and is ArrayBuffer
  if (!fileData || !(fileData instanceof ArrayBuffer)) {
    showError('Invalid file data received');
    console.error('[Cross-Origin D&D] Missing or invalid fileData');
    return;
  }

  // Check file size
  if (fileData.byteLength > MAX_FILE_SIZE) {
    showError(`File too large. Maximum size is ${MAX_FILE_SIZE / 1024 / 1024}MB`);
    console.error('[Cross-Origin D&D] File exceeds size limit:', fileData.byteLength);
    return;
  }

  // Validate file is not empty
  if (fileData.byteLength === 0) {
    showError('File is empty');
    console.error('[Cross-Origin D&D] Empty file received');
    return;
  }

  try {
    // Create a File object to match the native drag-drop interface
    // This allows us to reuse the existing processTonkFile() function
    const file = new File([fileData], fileName, {
      type: 'application/octet-stream',
      lastModified: Date.now()
    });

    console.log('[Cross-Origin D&D] Created File object, processing...', {
      name: file.name,
      size: file.size,
      type: file.type
    });

    // Process using existing bootloader logic
    // This function is defined in index.html
    if (typeof window.processTonkFile === 'function') {
      await window.processTonkFile(file);
      console.log('[Cross-Origin D&D] File processed successfully');

      // Send success response back to parent
      sendResponseToParent('success', fileName);
    } else {
      throw new Error('processTonkFile function not found');
    }
  } catch (error) {
    console.error('[Cross-Origin D&D] Error processing file:', error);
    showError('Failed to process file: ' + (error.message || 'Unknown error'));

    // Send error response back to parent
    sendResponseToParent('error', fileName, error.message || 'Unknown error');
  }
}

/**
 * Send response back to parent frame
 * @param {string} status - 'success' or 'error'
 * @param {string} fileName - Name of the file
 * @param {string} errorMessage - Error message if status is 'error'
 */
function sendResponseToParent(status, fileName, errorMessage = null) {
  // Only send if we're in an iframe
  if (window.parent === window) return;

  const response = {
    type: 'tonk:dropResponse',
    status,
    fileName
  };

  if (errorMessage) {
    response.error = errorMessage;
  }

  // Send to all allowed origins (parent will be one of them)
  ALLOWED_ORIGINS.forEach(origin => {
    try {
      window.parent.postMessage(response, origin);
      console.log('[Cross-Origin D&D] Sent response to parent:', response);
    } catch (err) {
      // Ignore errors for wrong origins
    }
  });
}

/**
 * Get the currently active boot menu element
 * Checks both prompt-screen and boot-screen for visible menu
 *
 * @returns {HTMLElement|null} The active boot menu or null
 */
function getActiveBootMenu() {
  return document.querySelector(
    '#prompt-screen:not([style*="display: none"]) .boot-menu, ' +
    '#boot-screen:not([style*="display: none"]) .boot-menu'
  );
}

/**
 * Show error message to user
 * Uses the existing showError function from index.html
 *
 * @param {string} message - Error message to display
 */
function showError(message) {
  if (typeof window.showError === 'function') {
    window.showError(message);
  } else {
    // Fallback if showError is not available
    console.error('[Cross-Origin D&D] Error:', message);
    alert(message);
  }
}

/**
 * Cleanup function to remove event listeners
 * Call this if you need to disable cross-origin drag-drop
 */
export function teardownCrossOriginDragDrop() {
  window.removeEventListener('message', handleMessage);
  console.log('[Cross-Origin D&D] Listener removed');
}
