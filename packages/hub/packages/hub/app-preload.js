<<<<<<< HEAD
const { contextBridge } = require('electron');

// Expose the Tonk Sync Bridge to the launched app
contextBridge.exposeInMainWorld('tonkSyncBridge', {
  // Document ID management functions
  setDocIdPrefix: (prefix) => {
    // This will be called by the main process via executeJavaScript
    if (typeof window.__TONK_SET_DOC_ID_PREFIX__ === 'function') {
      window.__TONK_SET_DOC_ID_PREFIX__(prefix);
    }
  },

  mapDocId: (logicalId, actualId) => {
    // This will be called by the main process via executeJavaScript
    if (typeof window.__TONK_MAP_DOC_ID__ === 'function') {
      window.__TONK_MAP_DOC_ID__(logicalId, actualId);
    }
  }
});
=======
const { contextBridge } = require('electron');

// Expose the Tonk Sync Bridge to the launched app
contextBridge.exposeInMainWorld('tonkSyncBridge', {
  // Document ID management functions
  setDocIdPrefix: (prefix) => {
    // This will be called by the main process via executeJavaScript
    if (typeof window.__TONK_SET_DOC_ID_PREFIX__ === 'function') {
      window.__TONK_SET_DOC_ID_PREFIX__(prefix);
    }
  },

  mapDocId: (logicalId, actualId) => {
    // This will be called by the main process via executeJavaScript
    if (typeof window.__TONK_MAP_DOC_ID__ === 'function') {
      window.__TONK_MAP_DOC_ID__(logicalId, actualId);
    }
  }
});
>>>>>>> Snippet

