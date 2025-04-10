const { app } = require('electron');
const path = require('node:path');
const fs = require('node:fs');

let _config = {};

const readConfig = () => {
  const appPath = app.getPath('userData');
  const configPath = path.join(appPath, 'config.json');
  console.log("THE CONFIG PATH IS", configPath);
  let configRaw = '{}';
  if (fs.existsSync(configPath)) {
    configRaw = fs.readFileSync(configPath);
  }
  _config = JSON.parse(configRaw);
  return _config;
}

const writeConfig = (content) => {
  const appPath = app.getPath('userData');
  _config = content;
  const configRaw = JSON.stringify(_config);
  fs.writeFileSync(path.join(appPath, 'config.json'), configRaw);
}

const getConfig = () => _config;

readConfig();

module.exports = {
  getConfig,
  writeConfig,
  readConfig
}