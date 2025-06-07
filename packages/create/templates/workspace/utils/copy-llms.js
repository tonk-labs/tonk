#!/usr/bin/env node

import fs from 'fs/promises';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// Define project subdirectories that should get their own .cursor/rules
// These are relative to the workspace root
const PROJECT_SUBDIRS = [
  'instructions',
  'workers', 
  'views',
  'console'
];

function generateMDCName(dirPath, workspaceRoot) {
  // Get the relative path from workspace root
  const relativePath = path.relative(workspaceRoot, dirPath);
  
  // Convert the directory path to a kebab-case name, removing special characters
  const name = relativePath
    .split(path.sep)
    .filter(Boolean)
    .join('-')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  
  return name ? `${name}-rules.mdc` : 'root-rules.mdc';
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

function findProjectRoot(currentPath, workspaceRoot) {
  // Normalize the paths to handle different OS path separators
  const normalizedCurrentPath = path.normalize(currentPath);
  const normalizedWorkspaceRoot = path.normalize(workspaceRoot);
  
  // Get relative path from workspace root
  const relativePath = path.relative(normalizedWorkspaceRoot, normalizedCurrentPath);
  
  // Check if this path is within any of our project subdirs
  for (const subdir of PROJECT_SUBDIRS) {
    if (relativePath.startsWith(subdir + path.sep) || relativePath === subdir) {
      return path.join(normalizedWorkspaceRoot, subdir);
    }
  }
  
  // If it's directly in the workspace root, return the workspace root
  if (relativePath === '' || !relativePath.includes(path.sep)) {
    return normalizedWorkspaceRoot;
  }
  
  return null;
}

async function copyLLMsContent(startPath, workspaceRoot = null) {
  // If workspaceRoot is not provided, use the startPath as workspace root
  if (!workspaceRoot) {
    workspaceRoot = startPath;
  }

  try {
    // Get all files and directories in the current path
    const items = await fs.readdir(startPath, { withFileTypes: true });

    for (const item of items) {
      const fullPath = path.join(startPath, item.name);

      if (item.isDirectory()) {
        // Skip node_modules and other common directories that shouldn't be processed
        if (['node_modules', '.git', '.next', 'dist', 'build'].includes(item.name)) {
          continue;
        }
        // Recursively process subdirectories
        await copyLLMsContent(fullPath, workspaceRoot);
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
        const projectRoot = findProjectRoot(dirPath, workspaceRoot);
        if (projectRoot) {
          try {
            const cursorRulesPath = await ensureCursorRulesDir(projectRoot);
            const mdcName = generateMDCName(dirPath, workspaceRoot);
            const mdcContent = generateMDCContent(content, dirPath, projectRoot);
            
            await fs.writeFile(path.join(cursorRulesPath, mdcName), mdcContent, 'utf8');
            console.log(`Created Cursor rule in ${projectRoot}: ${mdcName}`);
          } catch (error) {
            console.warn(`Warning: Could not create Cursor rule for ${dirPath}:`, error.message);
          }
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