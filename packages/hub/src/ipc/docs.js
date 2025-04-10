const { ipcMain } = require('electron');
const path = require('node:path');
const fs = require('node:fs');
const { getConfig } = require('../config.js');

const getFilePathForId = (docId) => {
  const { homePath } = getConfig();
  if (!homePath) {
    throw new Error(`Config is not properly set, found ${getConfig()}`);
  }
  const filePath = path.join(homePath, 'stores', `${docId}.automerge`);
  if (!fs.existsSync(filePath)) {
    throw new Error(`No file exists at path: ${filePath}`);
  }
}

const readDoc = async (docId) => {
  const filePath = getFilePathForId(docId);
  return new Promise((onSuccess, onError) => {
    // First try reading as UTF-8
    fs.readFile(filePath, 'utf8', (err, data) => {
      if (!err) {
        onSuccess(data);
        return;
      }
      onError(err);
    });
  });
};

ipcMain.handle('read-document', async (e, id) => {
  return await readDoc(id);
});

const writeFile = async (id, content) => {
  return new Promise((onSuccess, onError) => {
    const filePath = getFilePathForId(id);
    // Write the file
    fs.writeFile(filePath, content, (err) => {
      if (err) {
        onError(err);
        return;
      }
      onSuccess();
    });
  });
};

ipcMain.handle('save-document', async (e, docId, content) => {
  return await writeFile(docId, content);
});