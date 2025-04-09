const { ipcMain, shell } = require('electron');
const { getConfig } = require('../config.js');
const path = require('node:path');
const http = require('http');

ipcMain.handle('launch-app', async (event, projectPath) => {
  try {
    const distPath = path.join(projectPath, 'dist');
    // Set the distPath via API call
    const requestData = JSON.stringify({ distPath });
    
    return new Promise((resolve, reject) => {
      const req = http.request({
        hostname: 'localhost',
        port: 8080,
        path: '/api/toggle-dist-path',
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Content-Length': Buffer.byteLength(requestData)
        }
      }, (res) => {
        if (res.statusCode === 200) {
          shell.openExternal('http://localhost:8080');
          resolve(true);
        } else {
          reject(new Error(`Failed to set distPath: Status ${res.statusCode}`));
        }
      });
      
      req.on('error', (error) => {
        console.error('Error configuring server:', error);
        reject(error);
      });
      
      req.write(requestData);
      req.end();
    });
  } catch (error) {
    console.error('Error launching app:', error);
    throw error;
  }
});

ipcMain.handle('open-external-link', async (event, link) => {
  shell.openExternal(link);
})