const { ipcMain, webContents } = require('electron');
const chokidar = require('chokidar');
const path = require('node:path');
const { getConfig } = require('../config.js');

let watchers = new Map();

const createWatcher = (homePath, webContents) => {
  const watcher = chokidar.watch(homePath, {
    ignored: [
      '**/.git/**',
      '**/.vscode/**',
      '**/node_modules/**',
      '**/*.log',
      '**/Thumbs.db',
      '**/.DS_Store',
    ],
    persistent: true,
    depth: 3,
    ignoreInitial: true
  }).on('all', (event, path) => {
    console.log(event, path);
    webContents.send('file-change', { type: event, path })
  })
  return watcher;
};

ipcMain.handle('start-file-watching', async (event) => {
  const windowId = event.sender.id;
  
  // If we already have a watcher for this window, check if it's still working
  if (watchers.has(windowId)) {
    return true;
  }

  const homePath = getConfig().homePath;
  // Create new watcher only if we don't have a working one
  const watcher = createWatcher(homePath, webContents.fromId(windowId));
  watchers.set(windowId, watcher);
  
  return true;
});

ipcMain.handle('stop-file-watching', async (event) => {
  const windowId = event.sender.id;
  
  if (watchers.has(windowId)) {
    watchers.get(windowId).close();
    watchers.delete(windowId);
  }
  
  return true;
}); 