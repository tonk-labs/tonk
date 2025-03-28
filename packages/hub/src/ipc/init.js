const { ipcMain } = require('electron');
const { getConfig, writeConfig } = require('../config.js');
const path = require('node:path');

ipcMain.handle('init', async (e, homePath) => {
  const config = getConfig();
  writeConfig({
    ...config,
    homePath: path.join(homePath, '.tonk'),
  })
});