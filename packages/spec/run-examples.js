#!/usr/bin/env node

/**
 * Example Runner Script
 * 
 * Simple Node.js script to run the bundle analyzer examples.
 * This avoids the need to install tsx globally.
 */

import { spawn } from 'child_process';
import { fileURLToPath } from 'url';
import { dirname, join } from 'path';

const __dirname = dirname(fileURLToPath(import.meta.url));

function runExample(scriptName) {
  return new Promise((resolve, reject) => {
    console.log(`\n🚀 Running ${scriptName}...\n`);
    
    const child = spawn('npx', ['tsx', `examples/${scriptName}`], {
      cwd: __dirname,
      stdio: 'inherit',
      shell: true
    });

    child.on('close', (code) => {
      if (code === 0) {
        console.log(`\n✅ ${scriptName} completed successfully`);
        resolve();
      } else {
        reject(new Error(`${scriptName} failed with code ${code}`));
      }
    });

    child.on('error', (error) => {
      reject(error);
    });
  });
}

async function main() {
  const args = process.argv.slice(2);
  
  console.log('📦 Tonk Bundle Library Examples');
  console.log('================================\n');

  try {
    if (args.includes('--simple') || args.includes('-s')) {
      await runExample('simple-example.ts');
    } else if (args.includes('--both') || args.includes('-b')) {
      await runExample('simple-example.ts');
      await runExample('bundle-analyzer.ts');
    } else {
      // Default to comprehensive analyzer
      await runExample('bundle-analyzer.ts');
    }
    
    console.log('\n🎉 All examples completed successfully!');
  } catch (error) {
    console.error('\n❌ Error running examples:', error.message);
    process.exit(1);
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main();
}