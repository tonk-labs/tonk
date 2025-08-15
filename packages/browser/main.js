const { app, BrowserWindow, ipcMain, dialog } = require('electron');
const express = require('express');
const path = require('path');
const fs = require('fs');

const os = require('os');

let server;
let tonkServer;

// Helper function to read file content as Uint8Array (serializable over IPC)
const readFileContent = async filePath => {
  try {
    // Read as Buffer and convert to Uint8Array for IPC serialization
    const buffer = fs.readFileSync(filePath);
    const uint8Array = new Uint8Array(buffer);
    return {
      success: true,
      filePath,
      content: uint8Array,
      fileName: path.basename(filePath),
    };
  } catch (error) {
    console.error('Error reading file:', error);
    return {
      success: false,
      error: error.message,
      filePath,
    };
  }
};

// Function to start the Tonk sync server
const startTonkServer = async () => {
  try {
    // Set NODE_ENV to development to skip nginx setup
    process.env.NODE_ENV = 'development';

    // Import the server module dynamically since it's ESM
    const { createServer } = await import('@tonk/server');

    // Create directories for the sync server
    const appDataPath = path.join(os.homedir(), '.tonk-browser');
    const bundlesPath = path.join(appDataPath, 'bundles');
    const storesPath = path.join(appDataPath, 'stores');

    // Ensure directories exist
    if (!fs.existsSync(appDataPath)) {
      fs.mkdirSync(appDataPath, { recursive: true });
    }
    if (!fs.existsSync(bundlesPath)) {
      fs.mkdirSync(bundlesPath, { recursive: true });
    }
    if (!fs.existsSync(storesPath)) {
      fs.mkdirSync(storesPath, { recursive: true });
    }

    console.log('Starting Tonk sync server in development mode...');
    console.log(`Data directory: ${appDataPath}`);

    // Start the Tonk server on port 7777
    tonkServer = await createServer({
      port: 7777,
      persistencePath: appDataPath,
      bundlesPath,
      storesPath,
      verbose: true,
    });

    console.log('Tonk sync server started on port 7777');
    return true;
  } catch (error) {
    console.error('Failed to start Tonk sync server:', error);
    return false;
  }
};

let mainWindow;

const createWindow = async () => {
  // Start Tonk sync server first
  const tonkServerStarted = await startTonkServer();
  if (!tonkServerStarted) {
    console.warn(
      'Tonk sync server failed to start, continuing with browser only...'
    );
  }

  // Start local server
  const expressApp = express();
  expressApp.use(express.static(path.join(__dirname, 'dist')));
  server = expressApp.listen(0, () => {
    const port = server.address().port;
    console.log(`Local server started on port ${port}`);

    mainWindow = new BrowserWindow({
      fullscreen: false,
      webPreferences: {
        nodeIntegration: false,
        contextIsolation: true,
        preload: path.join(__dirname, 'preload.js'),
      },
    });

    mainWindow.loadURL(`http://localhost:${port}`);
  });
};

app.whenReady().then(createWindow);

// Function to send file data to renderer
async function sendFileToRenderer(filePath) {
  if (!mainWindow) return;

  const fileData = await readFileContent(filePath);
  mainWindow.webContents.send('file-received', fileData);
}

// IPC handlers for file operations
ipcMain.handle('open-file-dialog', async () => {
  const result = await dialog.showOpenDialog({
    properties: ['openFile'],
    filters: [
      { name: 'Tonk Apps', extensions: ['tonk'] },
      { name: 'All Files', extensions: ['*'] },
    ],
    title: 'Select a Tonk App Bundle',
  });

  if (!result.canceled && result.filePaths.length > 0) {
    await sendFileToRenderer(result.filePaths[0]);
    return { success: true, message: 'File sent to renderer' };
  }

  return null;
});

// Handle app termination - cleanup servers
app.on('before-quit', async () => {
  console.log('App is quitting, cleaning up servers...');

  // Stop the Tonk sync server
  if (tonkServer) {
    try {
      await tonkServer.stop();
      console.log('Tonk sync server stopped');
    } catch (error) {
      console.error('Error stopping Tonk sync server:', error);
    }
  }

  // Close the Express server
  if (server) {
    try {
      server.close(() => {
        console.log('Express server closed');
      });
    } catch (error) {
      console.error('Error closing Express server:', error);
    }
  }
});

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit();
  }
});

app.on('activate', () => {
  if (BrowserWindow.getAllWindows().length === 0) {
    createWindow();
  }
});

// Export function for external use
module.exports = { sendFileToRenderer };
