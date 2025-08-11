#!/usr/bin/env node

import fs from 'fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execSync } from 'node:child_process';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

/**
 * Content types that can exist in base locations
 * Using this as both runtime config and TypeScript type definition
 */
export const CONTENT_TYPES = {
  root: 'llms.txt',
  components: 'src/components/llms.txt',
  stores: 'src/stores/llms.txt',
  views: 'src/views/llms.txt',
  modules: 'src/modules/llms.txt',
  server: 'server/llms.txt',
  instructions: 'instructions/llms.txt',
  keepsync: 'instructions/keepsync/llms.txt',
  workers: 'workers/llms.txt',
};

/**
 * Special path overrides for locations that don't follow the standard pattern
 */
const PATH_OVERRIDES = {
  'examples/store-viewer': {
    instructions: 'src/instructions/llms.txt',
    keepsync: 'src/instructions/keepsync/llms.txt',
  },
  'packages/cli/template/apps/my-world': {
    instructions: 'src/instructions/llms.txt',
    keepsync: 'src/instructions/keepsync/llms.txt',
  },
};

/**
 * Source file mappings - which docs files map to which content types
 */
const SOURCE_MAPPINGS = {
  // Shared
  'docs/src/llms/shared/instructions.md': {
    contentTypes: ['root', 'instructions'],
  },
  'docs/src/llms/shared/components.md': {
    contentTypes: ['components'],
  },
  'docs/src/llms/shared/stores.md': {
    contentTypes: ['stores'],
  },
  'docs/src/llms/shared/views.md': {
    contentTypes: ['views'],
  },
  'docs/src/llms/shared/modules.md': {
    contentTypes: ['modules'],
  },
  'docs/src/llms/shared/server.md': {
    contentTypes: ['server'],
  },
  // Keepsync-specific
  'docs/src/llms/shared/keepsync/react-browser.md': {
    contentTypes: ['keepsync'],
  },
};

/**
 * Auto-discover base locations by scanning the filesystem
 */
async function discoverBaseLocations() {
  const baseLocations = [];
  const projectRoot = path.join(__dirname, '..');

  // Scan packages/create/templates/
  try {
    const templatesDir = path.join(projectRoot, 'packages/create/templates');
    const templates = await fs.readdir(templatesDir, { withFileTypes: true });

    for (const template of templates) {
      if (template.isDirectory()) {
        baseLocations.push(`packages/create/templates/${template.name}`);
      }
    }
  } catch (_error) {
    // Directory might not exist
  }

  // Scan packages/cli/template/apps/
  try {
    const cliTemplatesDir = path.join(
      projectRoot,
      'packages/cli/template/apps'
    );
    const cliTemplates = await fs.readdir(cliTemplatesDir, {
      withFileTypes: true,
    });

    for (const template of cliTemplates) {
      if (template.isDirectory()) {
        baseLocations.push(`packages/cli/template/apps/${template.name}`);
      }
    }
  } catch (_error) {
    // Directory might not exist
  }

  // Scan examples/
  try {
    const examplesDir = path.join(projectRoot, 'examples');
    const examples = await fs.readdir(examplesDir, { withFileTypes: true });

    for (const example of examples) {
      if (example.isDirectory()) {
        baseLocations.push(`examples/${example.name}`);
      }
    }
  } catch (_error) {
    // Directory might not exist
  }

  return baseLocations.sort();
}

/**
 * Generate target path for a given base location and content type
 */
function generateTargetPath(baseLocation, contentType) {
  // Check for path overrides first
  const overrides = PATH_OVERRIDES[baseLocation];
  if (overrides?.[contentType]) {
    return `${baseLocation}/${overrides[contentType]}`;
  }

  // Use default path from CONTENT_TYPES
  const defaultPath = CONTENT_TYPES[contentType];
  if (!defaultPath) {
    throw new Error(`Unknown content type: ${contentType}`);
  }

  return `${baseLocation}/${defaultPath}`;
}

/**
 * Check if a base location should be included for a source file
 */
function shouldIncludeLocation(baseLocation, sourceConfig) {
  // If include list exists, location must be in it
  if (sourceConfig.include) {
    return sourceConfig.include.some(pattern => baseLocation.includes(pattern));
  }

  // If exclude list exists, location must not be in it
  if (sourceConfig.exclude) {
    return !sourceConfig.exclude.some(pattern =>
      baseLocation.includes(pattern)
    );
  }

  // Default: include all locations
  return true;
}

/**
 * Auto-discover what content types should exist in a base location
 * Now returns all valid content types for the location, not just existing ones
 */
async function discoverContentTypes() {
  const availableTypes = [];

  // Always include all content types that are valid for this location
  // The distributor will create them if they don't exist
  for (const [contentType] of Object.entries(CONTENT_TYPES)) {
    availableTypes.push(contentType);
  }

  return availableTypes;
}

/**
 * Generate all target paths for a given source file
 */
async function generateTargetsForSource(sourceFile, baseLocations) {
  const sourceConfig = SOURCE_MAPPINGS[sourceFile];
  if (!sourceConfig) {
    console.warn(`No source mapping defined for: ${sourceFile}`);
    return [];
  }

  const targets = [];

  for (const baseLocation of baseLocations) {
    // Check if this location should be included for this source
    if (!shouldIncludeLocation(baseLocation, sourceConfig)) {
      continue;
    }

    // Get available content types for this location
    const availableTypes = await discoverContentTypes(baseLocation);

    // Generate targets for content types that exist and are mapped by this source
    for (const contentType of sourceConfig.contentTypes) {
      if (availableTypes.includes(contentType)) {
        targets.push(generateTargetPath(baseLocation, contentType));
      }
    }
  }

  return targets;
}

/**
 * Generate the complete distribution map
 */
async function generateDistributionMap() {
  const baseLocations = await discoverBaseLocations();
  const map = {};

  console.log(`📍 Discovered ${baseLocations.length} base locations:`);
  baseLocations.forEach(loc => console.log(`  - ${loc}`));

  for (const sourceFile of Object.keys(SOURCE_MAPPINGS)) {
    map[sourceFile] = await generateTargetsForSource(sourceFile, baseLocations);
  }

  return map;
}

// Generate the distribution map (this will be cached)
let _distributionMap = null;

/**
 * Get the distribution map (lazy loaded and cached)
 */
export async function getDistributionMap() {
  if (!_distributionMap) {
    _distributionMap = await generateDistributionMap();
  }
  return _distributionMap;
}

// For compatibility with synchronous usage, export a promise that resolves to the map
export const DISTRIBUTION_MAP = await generateDistributionMap();

/**
 * Files that should be generated from each llms.txt
 */
export const GENERATED_FILES = ['CLAUDE.md', '.cursorrules', '.windsurfrules'];

// Cache the docs version to avoid repeated git calls
let _docsVersion = null;

/**
 * Get version info from git for documentation files
 * Returns the short commit hash of the last change to docs/src/llms/
 * Cached to avoid multiple git calls during a single distribution run
 */
function getDocsVersion() {
  if (_docsVersion !== null) {
    return _docsVersion;
  }

  try {
    // Get the last commit that modified any file in docs/src/llms/
    const projectRoot = path.join(__dirname, '..');
    const commitHash = execSync('git log -1 --format="%h" -- docs/src/llms/', {
      cwd: projectRoot,
      encoding: 'utf8',
    }).trim();

    // Get the commit date for human readability
    const commitDate = execSync(
      `git log -1 --format="%cd" --date=short -- docs/src/llms/`,
      { cwd: projectRoot, encoding: 'utf8' }
    ).trim();

    _docsVersion = `${commitHash} (${commitDate})`;
  } catch (error) {
    // Fallback if git is not available or we're not in a git repo
    console.warn('Could not get git version info:', error.message);
    _docsVersion = 'unknown';
  }

  return _docsVersion;
}

export const HEADER_PREFIX = '# DocVersion:';

/**
 * Generate the header for distributed files
 */
export function generateHeader(sourceFile) {
  const version = getDocsVersion();
  return `# DO NOT EDIT - AUTO-GENERATED FROM ${sourceFile}
# This file is automatically generated from the documentation.
# Edit the source file instead: ${sourceFile}
${HEADER_PREFIX} ${version}

`;
}

/**
 * Get all target files for a given source
 */
export function getTargetsForSource(sourceFile) {
  return DISTRIBUTION_MAP[sourceFile] || [];
}

/**
 * Get all files that will be generated (llms.txt + generated files)
 */
export function getAllGeneratedFiles() {
  const allFiles = [];

  for (const targets of Object.values(DISTRIBUTION_MAP)) {
    for (const target of targets) {
      // Add the llms.txt file
      allFiles.push(target);

      // Add the generated files
      const dir = target.replace('/llms.txt', '');
      for (const genFile of GENERATED_FILES) {
        allFiles.push(`${dir}/${genFile}`);
      }
    }
  }

  return allFiles;
}

/**
 * Get summary statistics about the distribution
 */
export function getDistributionStats() {
  const sources = Object.keys(DISTRIBUTION_MAP);
  let totalTargets = 0;

  for (const targets of Object.values(DISTRIBUTION_MAP)) {
    totalTargets += targets.length;
  }

  return {
    sources: sources.length,
    targets: totalTargets,
    generatedFiles: totalTargets * GENERATED_FILES.length,
    totalFiles: totalTargets * (1 + GENERATED_FILES.length),
    contentTypes: Object.keys(CONTENT_TYPES).length,
  };
}

/**
 * Debug function to show what content types exist in each location
 */
export async function analyzeContentTypes() {
  const baseLocations = await discoverBaseLocations();
  const analysis = {};

  for (const location of baseLocations) {
    analysis[location] = await discoverContentTypes(location);
  }

  return analysis;
}

/**
 * Debug function to validate the mapping
 */
export function validateMapping() {
  const issues = [];

  for (const [source, targets] of Object.entries(DISTRIBUTION_MAP)) {
    if (targets.length === 0) {
      issues.push(`Source ${source} has no targets`);
    }

    for (const target of targets) {
      if (!target.endsWith('/llms.txt')) {
        issues.push(`Target ${target} doesn't end with /llms.txt`);
      }
    }
  }

  return issues;
}
