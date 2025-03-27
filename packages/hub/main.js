const { app, BrowserWindow, ipcMain, dialog } = require('electron');
const path = require('path');
const { spawn } = require('child_process');
const fs = require('fs');

// Keep a global reference of the window object to prevent garbage collection
let mainWindow;
let currentProcess = null;

// Path to store document IDs
const appDataPath = path.join(app.getPath('userData'), 'docIds.json');

// Function to load document IDs
function loadDocIds() {
  try {
    if (fs.existsSync(appDataPath)) {
      const data = fs.readFileSync(appDataPath, 'utf8');
      return JSON.parse(data);
    }
  } catch (error) {
    console.error('Error loading document IDs:', error);
  }
  return {};
}

// Function to save document IDs
function saveDocIds(docIds) {
  try {
    fs.writeFileSync(appDataPath, JSON.stringify(docIds, null, 2), 'utf8');
  } catch (error) {
    console.error('Error saving document IDs:', error);
  }
}

function createWindow() {
  // Create the browser window
  mainWindow = new BrowserWindow({
    width: 800,
    height: 600,
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      preload: path.join(__dirname, 'preload.js')
    },
    icon: path.join(__dirname, 'assets/icon.png')
  });

  // Load the index.html file
  mainWindow.loadFile('index.html');

  // Open DevTools in development mode
  if (process.env.NODE_ENV === 'development') {
    mainWindow.webContents.openDevTools();
  }

  // Handle window being closed
  mainWindow.on('closed', () => {
    if (currentProcess) {
      if (currentProcess.appWindow && !currentProcess.appWindow.isDestroyed()) {
        currentProcess.appWindow.close();
      }
      currentProcess.kill();
      currentProcess = null;
    }
    mainWindow = null;
  });
}

// Expose the current document ID to the renderer process
ipcMain.handle('get-current-doc-id', (event) => {
  if (currentProcess && currentProcess.appWindow) {
    return currentProcess.appWindow.docId || '';
  }
  return '';
});

// Create window when Electron has finished initialization
app.whenReady().then(() => {
  // Disable web security to allow loading of local resources
  app.commandLine.appendSwitch('disable-web-security');

  // Allow file access from file URLs
  app.commandLine.appendSwitch('allow-file-access-from-files');

  // Enable CORS for all origins
  app.commandLine.appendSwitch('disable-features', 'OutOfBlinkCors');

  // Allow loading of WASM files
  app.commandLine.appendSwitch('enable-features', 'SharedArrayBuffer');

  // Register file protocol handler for WASM files
  const protocol = require('electron').protocol;
  protocol.registerFileProtocol('wasm', (request, callback) => {
    const url = request.url.substr(7); // Remove 'wasm://' prefix
    callback({ path: url });
  });

  createWindow();

  // On macOS, re-create window when dock icon is clicked and no windows are open
  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createWindow();
    }
  });
});

// Quit when all windows are closed, except on macOS
app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit();
  }
});

// Handle selecting a project directory
ipcMain.handle('select-project', async () => {
  const result = await dialog.showOpenDialog(mainWindow, {
    properties: ['openDirectory'],
    title: 'Select Tonk Project Directory'
  });

  if (result.canceled) {
    return { success: false, message: 'Selection canceled' };
  }

  const projectPath = result.filePaths[0];
  return { success: true, path: projectPath };
});

// Get document IDs for a project
ipcMain.handle('get-project-doc-ids', async (event, projectPath) => {
  const docIds = loadDocIds();
  return docIds[projectPath] || [];
});

// Save a document ID for a project
ipcMain.handle('save-doc-id', async (event, projectPath, docId) => {
  if (!docId) return false;

  const docIds = loadDocIds();

  if (!docIds[projectPath]) {
    docIds[projectPath] = [];
  }

  // Only add if it doesn't already exist
  if (!docIds[projectPath].includes(docId)) {
    docIds[projectPath].push(docId);
    saveDocIds(docIds);
  }

  return true;
});

// Handle launching a Tonk app
ipcMain.handle('launch-app', async (event, projectPath, docId) => {
  // Kill any existing process and close any open app windows
  if (currentProcess) {
    if (currentProcess.appWindow && !currentProcess.appWindow.isDestroyed()) {
      currentProcess.appWindow.close();
    }
    currentProcess.kill();
    currentProcess = null;
  }

  // Send status to renderer about building
  mainWindow.webContents.send('app-status', { status: 'building', message: 'Building Tonk project...' });

  // First run npm run build
  const buildProcess = spawn('npm', ['run', 'build'], {
    cwd: projectPath,
    shell: true,
    env: {
      ...process.env,
      CI: 'true' // Skip interactive prompts
    }
  });

  // Log output from the build process
  buildProcess.stdout.on('data', (data) => {
    console.log(`[Build]: ${data}`);
    mainWindow.webContents.send('server-log', `[Build]: ${data.toString()}`);
  });

  buildProcess.stderr.on('data', (data) => {
    console.error(`[Build Error]: ${data}`);
    mainWindow.webContents.send('server-log', `[Build Error]: ${data.toString()}`);
  });

  // Wait for build to complete before starting the server
  buildProcess.on('close', (code) => {
    if (code !== 0) {
      console.error(`Build process exited with code ${code}`);
      mainWindow.webContents.send('app-status', {
        status: 'error',
        message: `Build failed with code ${code}`
      });
      return;
    }

    // Build succeeded, now start the Tonk server
    mainWindow.webContents.send('app-status', { status: 'loading', message: 'Starting Tonk server...' });

    // Start the Tonk server
    currentProcess = spawn('tonk', ['serve'], {
      cwd: projectPath,
      shell: true,
      env: {
        ...process.env,
        CI: 'true', // Skip interactive prompts
        TONK_DOC_ID: docId || '' // Pass the document ID as an environment variable
      }
    });

    // Log output from the server
    currentProcess.stdout.on('data', (data) => {
      console.log(`[Tonk Server]: ${data}`);
      mainWindow.webContents.send('server-log', data.toString());

      // Check if the server is ready and has a URL
      const output = data.toString();

      // Try to match URL patterns that might be in the output
      const urlPattern = /(https?:\/\/localhost:[0-9]+)/;
      const portPattern = /listening on port ([0-9]+)/i;

      let appUrl = null;

      // Try direct URL match
      const urlMatch = output.match(urlPattern);
      if (urlMatch) {
        appUrl = urlMatch[1];
      } else {
        // Try port match
        const portMatch = output.match(portPattern);
        if (portMatch) {
          const port = portMatch[1];
          appUrl = `http://localhost:${port}`;
        }
      }

      // If we found a URL and we haven't already opened a window, load it
      if (appUrl && !currentProcess.appWindow) {
        console.log(`Loading app from URL: ${appUrl}`);

        // Send status to renderer before navigating away
        mainWindow.webContents.send('app-status', {
          status: 'ready',
          message: 'Server is running',
          url: appUrl
        });

        // Create a new window for the Tonk app instead of replacing the main window
        currentProcess.appWindow = new BrowserWindow({
          width: 1024,
          height: 768,
          webPreferences: {
            nodeIntegration: false,
            contextIsolation: true,
            webSecurity: false, // Allow loading local resources
            allowRunningInsecureContent: true, // Allow loading mixed content
            preload: path.join(__dirname, 'tonk-preload.js'), // Add preload script to inject docId
            webviewTag: true, // Enable webview tag
            // Enable service workers
            serviceWorkers: true
          },
          icon: path.join(__dirname, 'assets/icon.png')
        });

        // Store the docId in the window object for the preload script to access
        currentProcess.appWindow.docId = docId;

        // Save the docId for this project if provided
        if (docId) {
          const docIds = loadDocIds();
          if (!docIds[projectPath]) {
            docIds[projectPath] = [];
          }
          if (!docIds[projectPath].includes(docId)) {
            docIds[projectPath].push(docId);
            saveDocIds(docIds);
          }
        }

        // Load the URL in the new window, adding docId as a query parameter if provided
        let loadUrl = appUrl;
        if (docId) {
          // Add docId as a query parameter
          const separator = loadUrl.includes('?') ? '&' : '?';
          loadUrl = `${loadUrl}${separator}docId=${encodeURIComponent(docId)}`;
        }
        currentProcess.appWindow.loadURL(loadUrl);

        // Open DevTools in development mode for the app window
        if (process.env.NODE_ENV === 'development') {
          currentProcess.appWindow.webContents.openDevTools();
        }

        // Handle app window being closed
        currentProcess.appWindow.on('closed', () => {
          // Focus back on the main window when app window is closed
          if (mainWindow) {
            mainWindow.focus();
          }
          // Clear the reference when the window is closed
          if (currentProcess) {
            currentProcess.appWindow = null;
          }
        });
      }
    });

    currentProcess.stderr.on('data', (data) => {
      console.error(`[Tonk Server Error]: ${data}`);
      mainWindow.webContents.send('server-log', data.toString());
      mainWindow.webContents.send('app-status', {
        status: 'error',
        message: `Error: ${data.toString()}`
      });
    });

    currentProcess.on('close', (code) => {
      console.log(`Tonk server process exited with code ${code}`);
      if (mainWindow) {
        mainWindow.webContents.send('app-status', {
          status: 'stopped',
          message: `Server stopped with code ${code}`
        });
      }
      currentProcess = null;
    });
  });

  return { success: true, message: 'Tonk server starting...' };
});
