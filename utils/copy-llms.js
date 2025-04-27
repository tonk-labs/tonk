#!/usr/bin/env node

import fs from 'fs/promises';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// Define project subdirectories that should get their own .cursor/rules
const PROJECT_SUBDIRS = [
  'packages/create/templates/react',
  'packages/create/templates/node',
];

function generateMDCName(dirPath) {
  // Convert the directory path to a kebab-case name, removing special characters
  const name = dirPath
    .split(path.sep)
    .filter(Boolean)
    .slice(-2) // Take last two parts of the path
    .join('-')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  
  return `${name}-rules.mdc`;
}

function generateMDCContent(content, dirPath, projectRoot) {
  // Convert the dirPath to be relative to the project root
  const relativePath = path.relative(projectRoot, dirPath);
  
  // Generate globs based on the relative directory path
  // If we're at the root, use *, otherwise use the relative path
  const globPrefix = relativePath === '' ? '*' : relativePath;
  const globs = `${globPrefix}/**/*.js, ${globPrefix}/**/*.ts, ${globPrefix}/**/*.tsx`;
  
  // Create metadata header
  const metadata = `---
description: Rules and guidelines for ${relativePath || 'root'}
globs: ${globs}
---

`;
  
  return metadata + content;
}

async function ensureCursorRulesDir(basePath) {
  const cursorRulesPath = path.join(basePath, '.cursor', 'rules');
  await fs.mkdir(cursorRulesPath, { recursive: true });
  return cursorRulesPath;
}

function findProjectRoot(currentPath) {
  // Normalize the path to handle different OS path separators
  const normalizedPath = currentPath.replace(/\\/g, '/');
  
  // Find if this path is within any of our project subdirs
  const matchedSubdir = PROJECT_SUBDIRS.find(subdir => 
    normalizedPath.includes(subdir)
  );
  
  if (!matchedSubdir) return null;
  
  // Get the project root by finding the index of the subdir and slicing up to it
  const subdirIndex = normalizedPath.indexOf(matchedSubdir);
  return normalizedPath.slice(0, subdirIndex + matchedSubdir.length);
}

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

        // Handle Cursor rules for project subdirs
        const projectRoot = findProjectRoot(dirPath);
        if (projectRoot) {
          const cursorRulesPath = await ensureCursorRulesDir(projectRoot);
          const mdcName = generateMDCName(dirPath);
          const mdcContent = generateMDCContent(content, dirPath, projectRoot);
          
          await fs.writeFile(path.join(cursorRulesPath, mdcName), mdcContent, 'utf8');
          console.log(`Created Cursor rule in ${projectRoot}: ${mdcName}`);
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