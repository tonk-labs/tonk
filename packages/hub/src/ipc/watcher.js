const { ipcMain, webContents } = require('electron');
const chokidar = require('chokidar');
const path = require('node:path');
const { getConfig } = require('../config.js');

let watchers = new Map();

const createWatcher = (homePath, webContents) => {
  // Watch the apps, stores, and integrations directories
  // const watchPaths = ['apps', 'stores', 'integrations'].map(dir => 
  //   path.join(homePath, dir, '**/*')
  // );

  // const watcher = chokidar.watch(watchPaths, {
  //   ignored: [
  //     /(^|[\/\\])\../, // Ignore dotfiles
  //     '**/node_modules/**',
  //     '**/*.log',
  //     '**/Thumbs.db',
  //     '**/.DS_Store',
  //   ],
  //   persistent: true,
  //   ignoreInitial: true,
  // });

  // // File events
  // watcher.on('add', path => {
  //   console.log("Detected file change!", path);
  //   webContents.send('file-change', { type: 'add', path });
  // });

  // watcher.on('change', path => {
  //   console.log("Detected file change!", path);
  //   webContents.send('file-change', { type: 'change', path });
  // });

  // watcher.on('unlink', path => {
  //   console.log("Detected file change!", path);
  //   webContents.send('file-change', { type: 'unlink', path });
  // });

  // // Directory events
  // watcher.on('addDir', path => {
  //   console.log("Detected file change!", path);
  //   webContents.send('file-change', { type: 'addDir', path });
  // });

  // watcher.on('unlinkDir', path => {
  //   console.log("Detected file change!", path);
  //   webContents.send('file-change', { type: 'unlinkDir', path });
  // });

  // // Add error handling
  // watcher.on('error', error => {
  //   console.error('Watcher error:', error);
  //   // Clean up the errored watcher
  //   const windowId = webContents.id;
  //   if (watchers.has(windowId)) {
  //     watchers.delete(windowId);
  //   }
  // });

  // return watcher;

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