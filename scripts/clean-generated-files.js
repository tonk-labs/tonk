#!/usr/bin/env node

import fs from 'node:fs';
import { getAllGeneratedFiles } from '../utils/file-mapping.js';

async function cleanGeneratedFiles() {
  console.log('Identifying auto-generated files...');

  const generatedFiles = getAllGeneratedFiles();
  console.log(`Found ${generatedFiles.length} auto-generated files to clean`);

  let deletedCount = 0;
  let skippedCount = 0;

  for (const file of generatedFiles) {
    if (fs.existsSync(file)) {
      try {
        fs.unlinkSync(file);
        deletedCount++;
        console.log(`Deleted: ${file}`);
      } catch (error) {
        console.error(`Failed to delete ${file}:`, error.message);
      }
    } else {
      skippedCount++;
    }
  }

  console.log(`\nCleanup complete:`);
  console.log(`- Deleted: ${deletedCount} files`);
  console.log(`- Skipped (didn't exist): ${skippedCount} files`);
  console.log(
    `\nNow run 'npm run docs:distribute' to regenerate with new versioning`
  );
}

cleanGeneratedFiles().catch(console.error);
