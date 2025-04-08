const { ipcMain } = require("electron");
const { getConfig, writeConfig } = require("../config.js");
const path = require("node:path");
const fs = require("fs-extra");
const { run } = require("../shell.js");
const { fetchRegistry } = require("../registry.js");

let wss;

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

ipcMain.handle("create-app", async (e, name) => {
    //this will create a new folder in apps
    //it will run tonk create app --init
    const child = require("child_process");
    // Assuming you're using npm start or similar to launch the app
    let config = getConfig();
    let appPath = path.join(config.homePath, "apps");
    child.exec(`cd "${appPath}" && mkdir ${name}`, (error, stdout, stderr) => {
        if (error) {
            console.error(`Error launching app: ${error}`);
            return;
        }
    });
});
