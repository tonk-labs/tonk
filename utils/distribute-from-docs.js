#!/usr/bin/env node

import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  DISTRIBUTION_MAP,
  GENERATED_FILES,
  generateHeader,
  getTargetsForSource,
} from "./file-mapping.js";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

/**
 * Read and process a docs source file
 */
async function readDocsSource(sourceFile) {
  const fullPath = path.join(__dirname, "..", sourceFile);

  try {
    const content = await fs.readFile(fullPath, "utf8");
    return content;
  } catch (error) {
    console.error(`Error reading source file ${sourceFile}:`, error.message);
    return null;
  }
}

/**
 * Read custom content from llms-custom.txt if it exists
 */
async function readCustomContent(targetFile) {
  const dir = path.dirname(targetFile);
  const customFile = path.join(dir, "llms-custom.txt");
  const fullCustomPath = path.join(__dirname, "..", customFile);

  try {
    const customContent = await fs.readFile(fullCustomPath, "utf8");
    return customContent.trim();
  } catch (_error) {
    // Custom file doesn't exist, which is fine
    return null;
  }
}

/**
 * Write content to a target file
 */
async function writeTargetFile(targetFile, content, sourceFile) {
  const fullPath = path.join(__dirname, "..", targetFile);
  const dir = path.dirname(fullPath);

  try {
    // Ensure directory exists
    await fs.mkdir(dir, { recursive: true });

    // Add header and write content
    const header = generateHeader(sourceFile);
    let fullContent = header + content;

    // Check for custom content and inject it
    const customContent = await readCustomContent(targetFile);
    if (customContent) {
      fullContent +=
        "\n\n---\n\n# Project-Specific Instructions\n\n" + customContent;
    }

    await fs.writeFile(fullPath, fullContent, "utf8");
    console.log(`✓ Distributed to: ${targetFile}`);
  } catch (error) {
    console.error(`✗ Error writing to ${targetFile}:`, error.message);
  }
}

/**
 * Generate the additional files (CLAUDE.md, .cursorrules, .windsurfrules)
 */
async function generateAdditionalFiles(targetFile, content, sourceFile) {
  const dir = path.dirname(targetFile);
  const header = generateHeader(sourceFile);
  let fullContent = header + content;

  // Check for custom content and inject it
  const customContent = await readCustomContent(targetFile);
  if (customContent) {
    fullContent +=
      "\n\n---\n\n# Project-Specific Instructions\n\n" + customContent;
  }

  for (const genFile of GENERATED_FILES) {
    const genPath = path.join(dir, genFile);
    const fullGenPath = path.join(__dirname, "..", genPath);

    try {
      await fs.writeFile(fullGenPath, fullContent, "utf8");
      console.log(`✓ Generated: ${genPath}`);
    } catch (error) {
      console.error(`✗ Error generating ${genPath}:`, error.message);
    }
  }
}

// Define project subdirectories that should get their own .cursor/rules
const PROJECT_SUBDIRS = [
  "packages/create/templates/react",
  "packages/create/templates/social-feed",
  "packages/create/templates/travel-planner",
];

function generateMDCName(dirPath) {
  // Convert the directory path to a kebab-case name, removing special characters
  const name = dirPath
    .split(path.sep)
    .filter(Boolean)
    .slice(-2) // Take last two parts of the path
    .join("-")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");

  return `${name}-rules.mdc`;
}

function generateMDCContent(content, dirPath, projectRoot) {
  // Convert the dirPath to be relative to the project root
  const relativePath = path.relative(projectRoot, dirPath);

  // Generate globs based on the relative directory path
  // If we're at the root, use *, otherwise use the relative path
  const globPrefix = relativePath === "" ? "*" : relativePath;
  const globs = `${globPrefix}/**/*.js, ${globPrefix}/**/*.ts, ${globPrefix}/**/*.tsx`;

  // Create metadata header
  const metadata = `---
description: Rules and guidelines for ${relativePath || "root"}
globs: ${globs}
---

`;

  return metadata + content;
}

async function ensureCursorRulesDir(basePath) {
  const cursorRulesPath = path.join(basePath, ".cursor", "rules");
  await fs.mkdir(cursorRulesPath, { recursive: true });
  return cursorRulesPath;
}

function findProjectRoot(currentPath) {
  // Normalize the path to handle different OS path separators
  const normalizedPath = currentPath.replace(/\\/g, "/");

  // Find if this path is within any of our project subdirs
  const matchedSubdir = PROJECT_SUBDIRS.find((subdir) =>
    normalizedPath.includes(subdir),
  );

  if (!matchedSubdir) return null;

  // Get the project root by finding the index of the subdir and slicing up to it
  const subdirIndex = normalizedPath.indexOf(matchedSubdir);
  return normalizedPath.slice(0, subdirIndex + matchedSubdir.length);
}

/**
 * Handle cursor rules generation (using existing logic)
 */
async function generateCursorRules(targetFile, content, sourceFile) {
  const dirPath = path.dirname(targetFile);
  const fullDirPath = path.join(__dirname, "..", dirPath);

  // Handle Cursor rules for project subdirs
  const projectRoot = findProjectRoot(fullDirPath);
  if (projectRoot) {
    const cursorRulesPath = await ensureCursorRulesDir(projectRoot);
    const mdcName = generateMDCName(fullDirPath);

    // Add header to content for cursor rules
    const header = generateHeader(sourceFile);
    const fullContent = header + content;

    // Check for custom content
    const customContent = await readCustomContent(targetFile);
    let mdcContent = generateMDCContent(fullContent, fullDirPath, projectRoot);

    // Inject custom content if it exists
    if (customContent) {
      mdcContent = generateMDCContent(
        fullContent +
          "\n\n---\n\n# Project-Specific Instructions\n\n" +
          customContent,
        fullDirPath,
        projectRoot,
      );
    }

    await fs.writeFile(path.join(cursorRulesPath, mdcName), mdcContent, "utf8");
    console.log(
      `✓ Generated cursor rule: ${path.relative(
        `${__dirname}/..`,
        projectRoot,
      )}/.cursor/rules/${mdcName}`,
    );
  }
}

/**
 * Distribute a single source file to all its targets
 */
async function distributeSingleSource(sourceFile) {
  console.log(`\n📄 Processing: ${sourceFile}`);

  const content = await readDocsSource(sourceFile);
  if (!content) {
    console.log(`❌ Skipping ${sourceFile} - could not read content`);
    return;
  }

  const targets = getTargetsForSource(sourceFile);
  if (targets.length === 0) {
    console.log(`❌ No targets found for ${sourceFile}`);
    return;
  }

  console.log(`📋 Distributing to ${targets.length} targets...`);

  for (const target of targets) {
    // Write the main llms.txt file
    await writeTargetFile(target, content, sourceFile);

    // Generate additional files
    await generateAdditionalFiles(target, content, sourceFile);

    // Handle cursor rules if needed
    await generateCursorRules(target, content, sourceFile);
  }
}

/**
 * Main distribution function
 */
async function distributeAll() {
  console.log("🚀 Starting distribution from docs to all targets...\n");

  const sources = Object.keys(DISTRIBUTION_MAP);
  console.log(`📚 Found ${sources.length} source files to distribute`);

  let totalTargets = 0;
  for (const targets of Object.values(DISTRIBUTION_MAP)) {
    totalTargets += targets.length;
  }
  console.log(`🎯 Total target locations: ${totalTargets}`);

  for (const sourceFile of sources) {
    await distributeSingleSource(sourceFile);
  }

  console.log("\n✅ Distribution complete!");
  console.log(`📄 Generated ${totalTargets} llms.txt files`);
  console.log(
    `📄 Generated ${totalTargets * GENERATED_FILES.length} additional files`,
  );
  console.log(
    `📄 Total files created: ${totalTargets * (1 + GENERATED_FILES.length)}`,
  );
}

/**
 * Validate that all source files exist
 */
async function validateSources() {
  console.log("🔍 Validating source files...");

  const sources = Object.keys(DISTRIBUTION_MAP);
  const missingFiles = [];

  for (const sourceFile of sources) {
    const fullPath = path.join(__dirname, "..", sourceFile);
    try {
      await fs.access(fullPath);
      console.log(`✓ ${sourceFile}`);
    } catch (_error) {
      console.log(`❌ ${sourceFile} - NOT FOUND`);
      missingFiles.push(sourceFile);
    }
  }

  if (missingFiles.length > 0) {
    console.error(`\n❌ Missing source files: ${missingFiles.length}`);
    missingFiles.forEach((file) => console.error(`  - ${file}`));
    return false;
  }

  console.log(`✅ All ${sources.length} source files exist\n`);
  return true;
}

/**
 * CLI interface
 */
async function main() {
  const args = process.argv.slice(2);

  if (args.includes("--help") || args.includes("-h")) {
    console.log(`
Usage: node distribute-from-docs.js [options]

Options:
  --validate, -v    Validate that all source files exist
  --help, -h        Show this help message

Examples:
  node distribute-from-docs.js              # Distribute all files
  node distribute-from-docs.js --validate   # Just validate sources
`);
    return;
  }

  if (args.includes("--validate") || args.includes("-v")) {
    await validateSources();
    return;
  }

  // Validate sources first
  const isValid = await validateSources();
  if (!isValid) {
    console.error(
      "\n❌ Validation failed. Fix missing files before distribution.",
    );
    process.exit(1);
  }

  // Run distribution
  await distributeAll();
}

// Only run if called directly
if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    console.error("Fatal error:", error);
    process.exit(1);
  });
}

export { distributeAll, validateSources, distributeSingleSource };
