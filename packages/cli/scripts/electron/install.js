#!/usr/bin/env node

import { execSync } from 'child_process';
import path from 'path';
import fs from 'fs';
import { fileURLToPath } from 'url';

// Get the current file's directory
const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// Get the path to the electron-app directory
const electronAppPath = path.resolve(__dirname, '../../../hub/');

// Check if the directory exists
if (!fs.existsSync(electronAppPath)) {
  console.error('Electron app directory not found!');
  process.exit(1);
}

console.log('Installing Electron app dependencies...');

try {
  // Change to the electron-app directory and install dependencies
  process.chdir(electronAppPath);
  execSync('npm install', { stdio: 'inherit' });

  console.log('Electron app dependencies installed successfully!');
} catch (error) {
  console.error(`Error installing Electron app dependencies: ${error.message}`);
  process.exit(1);
}
