#!/usr/bin/env node

import fs from 'fs/promises';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

async function copyLLMsContent(startPath) {
  try {
    // Get all files and directories in the current path
    const items = await fs.readdir(startPath, { withFileTypes: true });

    for (const item of items) {
      const fullPath = path.join(startPath, item.name);

      if (item.isDirectory()) {
        // Recursively process subdirectories
        await copyLLMsContent(fullPath);
      } else if (item.name === 'llms.txt') {
        console.log(`Found llms.txt at: ${fullPath}`);
        
        // Read the content of llms.txt
        const content = await fs.readFile(fullPath, 'utf8');
        const dirPath = path.dirname(fullPath);

        // Define target files
        const targetFiles = [
          path.join(dirPath, 'CLAUDE.md'),
          path.join(dirPath, '.cursorrules'),
          path.join(dirPath, '.windsurfrules')
        ];

        // Copy content to each target file
        for (const targetFile of targetFiles) {
          await fs.writeFile(targetFile, content, 'utf8');
          console.log(`Copied content to: ${targetFile}`);
        }
      }
    }
  } catch (error) {
    console.error('Error:', error.message);
    process.exit(1);
  }
}

// Start processing from the current directory if no path is provided
const startPath = process.argv[2] || process.cwd();
console.log(`Starting search from: ${startPath}`);

copyLLMsContent(startPath).catch(error => {
  console.error('Fatal error:', error);
  process.exit(1);
});