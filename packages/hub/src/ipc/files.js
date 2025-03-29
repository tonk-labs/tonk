const { ipcMain } = require('electron');
const path = require('node:path');
const fs = require('node:fs');

const readFile = async (filePath) => {
  if (!fs.existsSync(filePath)) {
    throw new Error(`No file exists at path: ${filePath}`);
  }

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

ipcMain.handle('read-file', async (e, filePath) => {
  return await readFile(filePath);
});

const writeFile = async (filePath, content) => {
  return new Promise((onSuccess, onError) => {
    // Ensure the directory exists
    const dir = path.dirname(filePath);
    fs.mkdir(dir, { recursive: true }, (mkdirErr) => {
      if (mkdirErr) {
        onError(mkdirErr);
        return;
      }
      // Write the file
      fs.writeFile(filePath, content, 'utf8', (err) => {
        if (err) {
          onError(err);
          return;
        }
        onSuccess();
      });
    });
  });
};

ipcMain.handle('write-file', async (e, filePath, content) => {
  return await writeFile(filePath, content);
});

const ls = async (dirPath) => {
  if (!fs.existsSync(dirPath)) {
    throw new Error(`Directory does not exist: ${dirPath}`);
  }
  return new Promise((onSuccess, onError) => {
    fs.readdir(dirPath, { withFileTypes: true }, (err, entries) => {
      if (err) {
        onError(err);
        return;
      }
      const results = entries.map(entry => ({
        name: entry.name,
        isDirectory: entry.isDirectory(),
        isFile: entry.isFile(),
        isSymlink: entry.isSymbolicLink()
      }));
      onSuccess(results);
    });
  });
};

ipcMain.handle('ls', async (e, dirPath) => {
  return await ls(dirPath);
});


const platformSensitiveJoin = async (paths) => {
  return path.join(...paths);
}

ipcMain.handle('platform-sensitive-join', async(e, paths) => {
  return platformSensitiveJoin(paths);
})