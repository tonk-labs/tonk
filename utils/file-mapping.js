#!/usr/bin/env node

import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

/**
 * Content types that can exist in base locations
 * Using this as both runtime config and TypeScript type definition
 */
export const CONTENT_TYPES = /** @type {const} */ ({
  root: "llms.txt",
  components: "src/components/llms.txt",
  stores: "src/stores/llms.txt",
  views: "src/views/llms.txt",
  modules: "src/modules/llms.txt",
  server: "server/llms.txt",
  instructions: "instructions/llms.txt",
  keepsync: "instructions/keepsync/llms.txt",
  workers: "workers/llms.txt",
});

/** @typedef {keyof typeof CONTENT_TYPES} ContentType */

/**
 * Special path overrides for locations that don't follow the standard pattern
 */
const PATH_OVERRIDES = {
  "examples/store-viewer": {
    instructions: "src/instructions/llms.txt",
    keepsync: "src/instructions/keepsync/llms.txt",
  },
  "packages/cli/template/apps/my-world": {
    instructions: "src/instructions/llms.txt",
    keepsync: "src/instructions/keepsync/llms.txt",
  },
  "packages/create/templates/workspace": {
    views: "views/llms.txt",
    workers: "workers/llms.txt",
  },
};

/**
 * Source file mappings - which docs files map to which content types
 */
const SOURCE_MAPPINGS = {
  "docs/src/llms/shared/instructions.md": {
    contentTypes: ["root", "instructions"],
    exclude: ["packages/create/templates/worker"], // Workers have their own instructions
  },
  "docs/src/llms/shared/components.md": {
    contentTypes: ["components"],
  },
  "docs/src/llms/shared/stores.md": {
    contentTypes: ["stores"],
  },
  "docs/src/llms/shared/views.md": {
    contentTypes: ["views"],
  },
  "docs/src/llms/shared/modules.md": {
    contentTypes: ["modules"],
  },
  "docs/src/llms/shared/server.md": {
    contentTypes: ["server"],
  },
  "docs/src/llms/shared/keepsync/react-browser.md": {
    contentTypes: ["keepsync"],
    exclude: ["packages/create/templates/worker"], // Workers use Node.js keepsync
  },
  "docs/src/llms/shared/keepsync/worker-nodejs.md": {
    contentTypes: ["keepsync"],
    include: ["packages/create/templates/worker"], // Only for worker templates
  },
  "docs/src/llms/templates/worker/README.md": {
    contentTypes: ["instructions"],
    include: ["packages/create/templates/worker"], // Only for worker templates
  },
  "docs/src/llms/templates/workspace/workers.md": {
    contentTypes: ["workers"],
    include: ["packages/create/templates/workspace"], // Only for workspace templates
  },
};

/**
 * Auto-discover base locations by scanning the filesystem
 */
async function discoverBaseLocations() {
  const baseLocations = [];
  const projectRoot = path.join(__dirname, "..");

  // Scan packages/create/templates/
  try {
    const templatesDir = path.join(projectRoot, "packages/create/templates");
    const templates = await fs.readdir(templatesDir, { withFileTypes: true });

    for (const template of templates) {
      if (template.isDirectory()) {
        baseLocations.push(`packages/create/templates/${template.name}`);
      }
    }
  } catch (error) {
    // Directory might not exist
  }

  // Scan packages/cli/template/apps/
  try {
    const cliTemplatesDir = path.join(
      projectRoot,
      "packages/cli/template/apps"
    );
    const cliTemplates = await fs.readdir(cliTemplatesDir, {
      withFileTypes: true,
    });

    for (const template of cliTemplates) {
      if (template.isDirectory()) {
        baseLocations.push(`packages/cli/template/apps/${template.name}`);
      }
    }
  } catch (error) {
    // Directory might not exist
  }

  // Scan examples/
  try {
    const examplesDir = path.join(projectRoot, "examples");
    const examples = await fs.readdir(examplesDir, { withFileTypes: true });

    for (const example of examples) {
      if (example.isDirectory()) {
        baseLocations.push(`examples/${example.name}`);
      }
    }
  } catch (error) {
    // Directory might not exist
  }

  return baseLocations.sort();
}

/**
 * Check if a file exists
 */
async function fileExists(filePath) {
  try {
    await fs.access(path.join(__dirname, "..", filePath));
    return true;
  } catch {
    return false;
  }
}

/**
 * Generate target path for a given base location and content type
 */
function generateTargetPath(baseLocation, contentType) {
  // Check for path overrides first
  const overrides = PATH_OVERRIDES[baseLocation];
  if (overrides && overrides[contentType]) {
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
    return sourceConfig.include.some((pattern) =>
      baseLocation.includes(pattern)
    );
  }

  // If exclude list exists, location must not be in it
  if (sourceConfig.exclude) {
    return !sourceConfig.exclude.some((pattern) =>
      baseLocation.includes(pattern)
    );
  }

  // Default: include all locations
  return true;
}

/**
 * Auto-discover what content types exist in a base location
 */
async function discoverContentTypes(baseLocation) {
  const availableTypes = [];

  for (const [contentType, _] of Object.entries(CONTENT_TYPES)) {
    const targetPath = generateTargetPath(baseLocation, contentType);
    if (await fileExists(targetPath)) {
      availableTypes.push(contentType);
    }
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
  baseLocations.forEach((loc) => console.log(`  - ${loc}`));

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
export const GENERATED_FILES = ["CLAUDE.md", ".cursorrules", ".windsurfrules"];

/**
 * Generate the header for distributed files
 */
export function generateHeader(sourceFile) {
  return `# DO NOT EDIT - AUTO-GENERATED FROM ${sourceFile}
# This file is automatically generated from the documentation.
# Edit the source file instead: ${sourceFile}
# 
# Generated on: ${new Date().toISOString()}
# 

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
      const dir = target.replace("/llms.txt", "");
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
      if (!target.endsWith("/llms.txt")) {
        issues.push(`Target ${target} doesn't end with /llms.txt`);
      }
    }
  }

  return issues;
}
