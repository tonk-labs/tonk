const { ipcMain } = require('electron');
const { getConfig, writeConfig } = require('../config.js');
const path = require('node:path');
const fs = require('fs-extra');
const { run } = require('../shell.js') 

let wss;

ipcMain.handle('init', async (e, homePath) => {
  const config = getConfig();
  writeConfig({
    ...config,
    homePath: path.join(homePath, '.tonk'),
  })
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