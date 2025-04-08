const { ipcMain } = require("electron");
const { getConfig, writeConfig } = require("../config.js");
const path = require("node:path");
const fs = require("fs-extra");
const { run } = require("../shell.js");
const { fetchRegistry } = require("../registry.js");

let wss;
let server;

ipcMain.handle("init", async (e, homePath) => {
    const config = getConfig();
    writeConfig({
        ...config,
        homePath: path.join(homePath, ".tonk"),
    });
});

ipcMain.handle("fetch-registry", async () => {
    try {
        const registry = await fetchRegistry();
        return { success: true, data: registry };
    } catch (error) {
        console.error("Failed to fetch registry:", error);
        return { success: false, error: error.message };
    }
  })
ipcMain.handle('clear-config', async (e) => {
  writeConfig({});
});

ipcMain.handle('copy-hub-template', async (e) => {
  const config = getConfig();
  const templatePath = path.join(__dirname, '../../template');
  const targetPath = path.join(config.homePath);
  
  try {
    await fs.ensureDir(path.dirname(targetPath));
    await fs.copy(templatePath, targetPath);
    return { success: true };
  } catch (error) {
    console.error('Failed to copy template:', error);
    return { success: false, error: error.message };
  }
})

ipcMain.handle('run-shell', async (e, dirPath) => {
  if (!wss) {
    wss = run(3060, dirPath);
  }
})

ipcMain.handle('close-shell', async () =>{
  if (wss) {
    wss.close()
    wss = null;
  }
})

ipcMain.handle('create-app', async (e, name) => {
  //this will create a new folder in apps
  //it will run tonk create app --init
  const child = require('child_process');
  // Assuming you're using npm start or similar to launch the app
  let config = getConfig();
  let appPath = path.join(config.homePath, 'apps');
  child.exec(`cd "${appPath}" && mkdir ${name}`, (error, stdout, stderr) => {
    if (error) {
      console.error(`Error launching app: ${error}`);
      return;
    }
  });
})

ipcMain.handle('run-server', async (event, restart = false) => {
  try {
    if (!!server && !restart) {
      console.log('We already have a server!')
      return;
    } else if (server && restart) {
      server.kill(0);
      server = null;
    }
    // Launch the app with the docId as a query parameter
    // You'll need to adjust this based on how your app is actually launched
    const child = require('child_process');
    // Assuming you're using npm start or similar to launch the app
    let config = getConfig();
    let storesPath = path.join(config.homePath, 'stores');
    server = child.exec(`tonk serve -u -f ${storesPath}`, {
      cwd: config.homePath,
      
      // Ensure we get string output instead of buffers
      encoding: 'utf8',
      // Increase buffer size to avoid truncation
      maxBuffer: 1024 * 1024 * 10
    }, (error, stdout, stderr) => {
      if (error) {
        console.error(`Error launching app: ${error}`);
        return;
      }
    });

    // Handle output streams directly
    server.stdout.on('data', (data) => {
      console.log(`[Server] ${data.toString().trim()}`);
    });

    server.stderr.on('data', (data) => {
      console.error(`[Server Error] ${data.toString().trim()}`);
    });

    server.on('close', (code) => {
      console.log(`Server process exited with code ${code}`);
      server = null;
    });

    return true;
  } catch (error) {
    console.error('Error launching app:', error);
    throw error;
  }
});
