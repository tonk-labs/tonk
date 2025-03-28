// main.js

// Modules to control application life and create native browser window
const { app, BrowserWindow, protocol, ipcMain } = require('electron')
const path = require('node:path')
const { spawn } = require('child_process')
const fs = require('fs')
require('@electron/remote/main').initialize()

// Add document ID storage path
const appDataPath = path.join(app.getPath('userData'), 'docIds.json');

// Add document ID management functions
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

function saveDocIds(docIds) {
  try {
    fs.writeFileSync(appDataPath, JSON.stringify(docIds, null, 2), 'utf8');
  } catch (error) {
    console.error('Error saving document IDs:', error);
  }
}

let createProtocol = (scheme, normalize = true) => {
  protocol.registerBufferProtocol(scheme,
    (request, respond) => {
      let pathName = new URL(request.url).pathname;

      // Needed in case URL contains spaces
      pathName = decodeURI(pathName);

      readFile(__dirname + "/" + pathName, (error, data) => {
        let extension = extname(pathName).toLowerCase();
        let mimeType = "";
        if (extension === ".js") {
          mimeType = "text/javascript";
        } else if (extension === ".html") {
          mimeType = "text/html";
        } else if (extension === ".css") {
          mimeType = "text/css";
        } else if (extension === ".svg" || extension ===
          ".svgz") {
          mimeType = "image/svg+xml";
        } else if (extension === ".json") {
          mimeType = "application/json";
        }
        respond({
          mimeType,
          data
        });
      });
    },
    (error) => {
      if (error) {
        console.error(`Failed to register ${scheme} protocol`,
          error);
      }
    }
  );
}

// Standard scheme must be registered before the app is ready
// https://gist.github.com/dbkr/e898624be6d53590ebf494521d868fec
protocol.registerSchemesAsPrivileged([{
  scheme: 'app',
  privileges: { standard: true, secure: true, supportFetchAPI: true },
}]);

let mainWindow;
let nodemonProcess;
let currentAppWindow = null;

const loadApp = (appPath, docId) => {
  if (!mainWindow) {
    console.error('Window not initialized');
    return;
  }

  // Kill existing nodemon process if it exists
  if (nodemonProcess) {
    nodemonProcess.kill();
    nodemonProcess = null;
  }

  // Start nodemon to watch for changes
  const nodemonPath = path.join(process.cwd(), 'node_modules', '.bin', 'nodemon');
  const nodemonConfig = path.join(process.cwd(), 'nodemon.json');
  const appRoot = path.join(appPath, '..');
  nodemonProcess = spawn(nodemonPath, ['--config', nodemonConfig], {
    cwd: path.dirname(appRoot),
    stdio: 'inherit'
  });

  nodemonProcess.on('error', (err) => {
    console.error('Failed to start nodemon:', err);
  });

  // Create a new window for the app
  currentAppWindow = new BrowserWindow({
    width: 1024,
    height: 800,
    webPreferences: {
      nodeIntegration: true,
      contextIsolation: false,
      webSecurity: false,
      allowRunningInsecureContent: true,
      webgl: true,
      enableRemoteModule: true,
      experimentalFeatures: true,
    }
  });

  // Store the docId in the window object
  currentAppWindow.docId = docId;

  // Modify the URL to include docId if provided
  let loadUrl = `file://${appPath}`;
  if (docId) {
    loadUrl += `?docId=${encodeURIComponent(docId)}`;
  }

  currentAppWindow.loadURL(loadUrl);
  currentAppWindow.maximize();
}

const createWindow = () => {
  // Create the browser window.
  mainWindow = new BrowserWindow({
    width: 1024,
    height: 800,
    webPreferences: {
      nodeIntegration: true,
      contextIsolation: false,
      preload: path.join(__dirname, 'preload.js'),
      webSecurity: false,
      allowRunningInsecureContent: true,
      webgl: true,
      enableRemoteModule: true,
      // Enable WebAssembly
      experimentalFeatures: true,
    }
  })

  // Enable @electron/remote for this window
  require('@electron/remote/main').enable(mainWindow.webContents)

  // // Load the app
  // const appPath = path.join(__dirname, 'index.html')
  mainWindow.loadFile('index.html');

  // Optional: Log any unhandled rejections for debugging
  process.on('unhandledRejection', (reason, promise) => {
    console.log('Unhandled Rejection at:', promise, 'reason:', reason)
  })

  mainWindow.maximize();
  // mainWindow.webContents.openDevTools();
}

// Add new IPC handlers for document ID functionality
ipcMain.handle('get-current-doc-id', (event) => {
  if (currentAppWindow) {
    return currentAppWindow.docId || '';
  }
  return '';
});

ipcMain.handle('get-project-doc-ids', async (event, projectPath) => {
  const docIds = loadDocIds();
  return docIds[projectPath] || [];
});

ipcMain.handle('save-doc-id', async (event, projectPath, docId) => {
  if (!docId) return false;

  const docIds = loadDocIds();
  if (!docIds[projectPath]) {
    docIds[projectPath] = [];
  }

  if (!docIds[projectPath].includes(docId)) {
    docIds[projectPath].push(docId);
    saveDocIds(docIds);
  }

  return true;
});

// Modify the existing IPC handler for folder selection
ipcMain.on('selected-folder', (event, _path, docId) => {
  console.log('Selected folder:', _path, 'Document ID:', docId);
  const appPath = path.join(_path, 'dist/index.html');

  // Save the document ID if provided
  if (docId) {
    const projectPath = _path;
    const docIds = loadDocIds();
    if (!docIds[projectPath]) {
      docIds[projectPath] = [];
    }
    if (!docIds[projectPath].includes(docId)) {
      docIds[projectPath].push(docId);
      saveDocIds(docIds);
    }
  }

  loadApp(appPath, docId);
});

// Handle app launching with docId
ipcMain.handle('launch-app', async (event, projectPath, docId) => {
  try {
    // Save the docId for this project
    const docIds = loadDocIds();
    if (!docIds[projectPath]) {
      docIds[projectPath] = [];
    }
    if (!docIds[projectPath].includes(docId)) {
      docIds[projectPath].unshift(docId);
      // Keep only the last 5 docIds
      docIds[projectPath] = docIds[projectPath].slice(0, 5);
    }
    fs.writeFileSync(appDataPath, JSON.stringify(docIds));

    // Launch the app with the docId as a query parameter
    // You'll need to adjust this based on how your app is actually launched
    const child = require('child_process');
    // Assuming you're using npm start or similar to launch the app
    child.exec(`cd "${projectPath}" && npm start -- --docId=${docId}`, (error, stdout, stderr) => {
      if (error) {
        console.error(`Error launching app: ${error}`);
        return;
      }
      console.log(`App launched with docId: ${docId}`);
    });

    return true;
  } catch (error) {
    console.error('Error launching app:', error);
    throw error;
  }
});

// This method will be called when Electron has finished
// initialization and is ready to create browser windows.
// Some APIs can only be used after this event occurs.
app.whenReady().then(() => {
  // Register protocol handler for serving local files
  createProtocol("app");


  createWindow()

  app.on('activate', () => {
    // On macOS it's common to re-create a window in the app when the
    // dock icon is clicked and there are no other windows open.
    if (BrowserWindow.getAllWindows().length === 0) createWindow()
  })
})

// Quit when all windows are closed, except on macOS. There, it's common
// for applications and their menu bar to stay active until the user quits
// explicitly with Cmd + Q.
app.on('window-all-closed', () => {
  // Kill nodemon process when app is closing
  if (nodemonProcess) {
    nodemonProcess.kill();
    nodemonProcess = null;
  }

  if (currentAppWindow && !currentAppWindow.isDestroyed()) {
    currentAppWindow.close();
    currentAppWindow = null;
  }

  if (process.platform !== 'darwin') app.quit()
})

// In this file you can include the rest of your app's specific main process
// code. You can also put them in separate files and require them here.
