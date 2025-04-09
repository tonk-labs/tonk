// Modules to control application life and create native browser window
const { app, BrowserWindow, protocol, ipcMain } = require("electron");
const path = require("node:path");
const process = require("node:process");
const ngrok = require("ngrok");

const { getConfig, readConfig } = require("./src/config.js");
require("@electron/remote/main").initialize();
// Import app.js directly to get access to the appWindow variable
const appModule = require("./src/ipc/app.js");
require("./src/ipc/hub.js");
require("./src/ipc/integrations.js");
require("./src/ipc/files.js");
require("./src/ipc/watcher.js");

let createProtocol = (scheme, normalize = true) => {
    protocol.registerBufferProtocol(
        scheme,
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
                } else if (extension === ".svg" || extension === ".svgz") {
                    mimeType = "image/svg+xml";
                } else if (extension === ".json") {
                    mimeType = "application/json";
                }
                respond({
                    mimeType,
                    data,
                });
            });
        },
        (error) => {
            if (error) {
                console.error(`Failed to register ${scheme} protocol`, error);
            }
        }
    );
};

// Standard scheme must be registered before the app is ready
// https://gist.github.com/dbkr/e898624be6d53590ebf494521d868fec
protocol.registerSchemesAsPrivileged([
    {
        scheme: "app",
        privileges: { standard: true, secure: true, supportFetchAPI: true },
    },
]);

let mainWindow;
let nodemonProcess;

const createWindow = () => {
    // Create the browser window.
    mainWindow = new BrowserWindow({
        width: 1024,
        height: 800,
        title: "Tonk",
        titleBarStyle: "hidden", // Hide title bar on macOS but keep traffic lights
        frame: process.platform !== "darwin", // Use native frame on macOS, frameless on Windows
        titleBarOverlay: process.platform !== "darwin", // Enable system buttons on Windows
        backgroundColor: "#131313",
        webPreferences: {
            nodeIntegration: true,
            contextIsolation: true,
            preload: path.join(__dirname, "preload.js"),
            webSecurity: false,
            allowRunningInsecureContent: true,
            webgl: true,
            enableRemoteModule: true,
            // Enable WebAssembly
            experimentalFeatures: true,
        },
    });

    // Enable @electron/remote for this window
    require("@electron/remote/main").enable(mainWindow.webContents);

    // // Load the app
    // const appPath = path.join(__dirname, 'index.html')
    if (process.env.DEVELOPMENT_MODE == "true") {
        mainWindow.loadURL("http://localhost:3333");
    } else {
        mainWindow.loadFile("index.html");
    }

    // Optional: Log any unhandled rejections for debugging
    process.on("unhandledRejection", (reason, promise) => {
        console.log("Unhandled Rejection at:", promise, "reason:", reason);
    });

    // mainWindow.webContents.openDevTools();
};

ipcMain.handle("get-config", async () => {
    return getConfig();
});

// This method will be called when Electron has finished
// initialization and is ready to create browser windows.
// Some APIs can only be used after this event occurs.
app.whenReady().then(() => {
    // Register protocol handler for serving local files
    createProtocol("app");

    createWindow();

    app.on("activate", () => {
        // On macOS it's common to re-create a window in the app when the
        // dock icon is clicked and there are no other windows open.
        if (BrowserWindow.getAllWindows().length === 0) createWindow();
    });
});

// Quit when all windows are closed, except on macOS. There, it's common
// for applications and their menu bar to stay active until the user quits
// explicitly with Cmd + Q.
app.on("window-all-closed", async () => {
    // Kill nodemon process when app is closing
    if (ngrok.isRunning()) {
        await ngrok.disconnect();
        await ngrok.kill();
    }

    if (nodemonProcess) {
        nodemonProcess.kill();
        nodemonProcess = null;
    }

    if (appModule.appWindow && !appModule.appWindow.isDestroyed()) {
        appModule.appWindow.close();
    }

    if (process.platform !== "darwin") app.quit();
});

// In this file you can include the rest of your app's specific main process
// code. You can also put them in separate files and require them here.
