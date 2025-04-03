import fs from 'fs-extra';
import path from 'node:path';
import packageJSON from '../package.json' assert { type: 'json' };
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

async function main() {
  try {
    const rootDir = process.env.INIT_CWD;
    const targetDir = path.join(rootDir, 'documentation', 'integrations', packageJSON.name); 

    // Create target directory if it doesn't exist
    if (!fs.existsSync(targetDir)) {
      fs.mkdirSync(targetDir, { recursive: true });
    }

    // Source directory where your files are stored in your package
    const sourceDir = path.join(__dirname, '..', 'files'); // Adjust the path as needed
    const cursorRulesPath = path.join(__dirname, '..', '.cursorrules');
    const windsurfRulesPath = path.join(__dirname, '..', '.windsurfrules');

    // Copy the files directory
    fs.copySync(sourceDir, targetDir);

    // Copy rule files if they exist
    if (fs.existsSync(cursorRulesPath)) {
      fs.copySync(cursorRulesPath, path.join(targetDir, '.cursorrules'));
    }
    if (fs.existsSync(windsurfRulesPath)) {
      fs.copySync(windsurfRulesPath, path.join(targetDir, '.windsurfrules'));
    }
    console.log('postinstall complete!');
  } catch (err) {
    console.error('Error copying files:', err);
    process.exit(1);
  }
}

main();