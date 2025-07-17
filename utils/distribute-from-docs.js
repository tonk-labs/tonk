#!/usr/bin/env node

import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  DISTRIBUTION_MAP,
  GENERATED_FILES,
  generateHeader,
  getTargetsForSource,
  getAllGeneratedFiles,
} from "./file-mapping.js";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

/**
 * Convert markdown content to plain text for llms.txt files
 */
function convertMarkdownToPlainText(markdownContent) {
  // Remove markdown headers (convert ## Header to just Header)
  let content = markdownContent.replace(/^#+\s+/gm, "");

  // Remove markdown code fences but keep the content
  content = content.replace(/```[\s\S]*?\n([\s\S]*?)```/g, "```\n$1```");

  // Remove markdown links but keep the text
  content = content.replace(/\[([^\]]+)\]\([^\)]+\)/g, "$1");

  // Remove markdown bold/italic
  content = content.replace(/\*\*([^*]+)\*\*/g, "$1");
  content = content.replace(/\*([^*]+)\*/g, "$1");

  // Remove markdown include directives (mdBook specific)
  content = content.replace(/\{\{#include\s+[^}]+\}\}/g, "");

  // Clean up excessive whitespace
  content = content.replace(/\n\s*\n\s*\n/g, "\n\n");

  return content.trim();
}

/**
 * Read and process a docs source file
 */
async function readDocsSource(sourceFile) {
  const fullPath = path.join(__dirname, "..", sourceFile);

  try {
    const content = await fs.readFile(fullPath, "utf8");
    return convertMarkdownToPlainText(content);
  } catch (error) {
    console.error(`Error reading source file ${sourceFile}:`, error.message);
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
    const fullContent = header + content;

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
  const fullContent = header + content;

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

/**
 * Handle cursor rules generation (using existing logic)
 */
async function generateCursorRules(targetFile, content, sourceFile) {
  // This will integrate with the existing copy-llms.js logic for PROJECT_SUBDIRS
  // For now, we'll skip this and let the existing script handle it
  return;
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
    `📄 Generated ${totalTargets * GENERATED_FILES.length} additional files`
  );
  console.log(
    `📄 Total files created: ${totalTargets * (1 + GENERATED_FILES.length)}`
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
    } catch (error) {
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
      "\n❌ Validation failed. Fix missing files before distribution."
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
