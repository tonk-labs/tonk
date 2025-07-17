#!/usr/bin/env node

import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { DISTRIBUTION_MAP, generateHeader } from "./file-mapping.js";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

/**
 * Convert markdown content to plain text for comparison
 */
function convertMarkdownToPlainText(markdownContent) {
  // Same logic as distribute-from-docs.js
  let content = markdownContent.replace(/^#+\s+/gm, "");
  content = content.replace(/```[\s\S]*?\n([\s\S]*?)```/g, "```\n$1```");
  content = content.replace(/\[([^\]]+)\]\([^\)]+\)/g, "$1");
  content = content.replace(/\*\*([^*]+)\*\*/g, "$1");
  content = content.replace(/\*([^*]+)\*/g, "$1");
  content = content.replace(/\{\{#include\s+[^}]+\}\}/g, "");
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
    return null;
  }
}

/**
 * Read a target file and extract the content (removing header)
 */
async function readTargetFile(targetFile) {
  const fullPath = path.join(__dirname, "..", targetFile);

  try {
    const content = await fs.readFile(fullPath, "utf8");

    // Remove the auto-generated header
    const lines = content.split("\n");
    let contentStart = 0;

    // Find the end of the header (look for the empty line after "# Generated on:")
    for (let i = 0; i < lines.length; i++) {
      if (lines[i].startsWith("# Generated on:")) {
        // Content starts after the next empty line
        contentStart = i + 2;
        break;
      }
    }

    return lines.slice(contentStart).join("\n").trim();
  } catch (error) {
    return null;
  }
}

/**
 * Check if a target file is in sync with its source
 */
async function checkFileSync(sourceFile, targetFile) {
  const sourceContent = await readDocsSource(sourceFile);
  const targetContent = await readTargetFile(targetFile);

  if (sourceContent === null) {
    return { synced: false, reason: "Source file not found" };
  }

  if (targetContent === null) {
    return { synced: false, reason: "Target file not found" };
  }

  // Check if target has auto-generated header
  const targetFullContent = await fs.readFile(
    path.join(__dirname, "..", targetFile),
    "utf8"
  );
  if (!targetFullContent.includes("DO NOT EDIT - AUTO-GENERATED")) {
    return { synced: false, reason: "Missing auto-generated header" };
  }

  // Check content equality
  if (sourceContent !== targetContent) {
    return {
      synced: false,
      reason: "Content mismatch",
      sourceLength: sourceContent.length,
      targetLength: targetContent.length,
    };
  }

  return { synced: true };
}

/**
 * Validate all files are in sync
 */
async function validateAllFiles() {
  console.log("🔍 Validating that all files are in sync with docs...\n");

  let totalFiles = 0;
  let syncedFiles = 0;
  let outOfSyncFiles = 0;
  const issues = [];

  for (const [sourceFile, targets] of Object.entries(DISTRIBUTION_MAP)) {
    console.log(`📄 Checking ${sourceFile}...`);

    for (const targetFile of targets) {
      totalFiles++;
      const result = await checkFileSync(sourceFile, targetFile);

      if (result.synced) {
        console.log(`  ✅ ${targetFile}`);
        syncedFiles++;
      } else {
        console.log(`  ❌ ${targetFile} - ${result.reason}`);
        outOfSyncFiles++;
        issues.push({
          source: sourceFile,
          target: targetFile,
          reason: result.reason,
          sourceLength: result.sourceLength,
          targetLength: result.targetLength,
        });
      }
    }
  }

  console.log("\n📊 Validation Results:");
  console.log(`  📄 Total files: ${totalFiles}`);
  console.log(`  ✅ In sync: ${syncedFiles}`);
  console.log(`  ❌ Out of sync: ${outOfSyncFiles}`);

  if (outOfSyncFiles > 0) {
    console.log("\n❌ Issues found:");
    for (const issue of issues) {
      console.log(`  - ${issue.target}`);
      console.log(`    Source: ${issue.source}`);
      console.log(`    Reason: ${issue.reason}`);
      if (issue.sourceLength && issue.targetLength) {
        console.log(
          `    Length: source=${issue.sourceLength}, target=${issue.targetLength}`
        );
      }
    }

    console.log("\n💡 To fix these issues, run:");
    console.log("   node utils/distribute-from-docs.js");

    return false;
  }

  console.log("\n✅ All files are in sync!");
  return true;
}

/**
 * Check if any files have been manually edited
 */
async function detectManualEdits() {
  console.log("🔍 Checking for manual edits to generated files...\n");

  let totalFiles = 0;
  let manualEdits = 0;
  const editedFiles = [];

  for (const targets of Object.values(DISTRIBUTION_MAP)) {
    for (const targetFile of targets) {
      totalFiles++;
      const fullPath = path.join(__dirname, "..", targetFile);

      try {
        const content = await fs.readFile(fullPath, "utf8");

        // Check if file has auto-generated header
        if (!content.includes("DO NOT EDIT - AUTO-GENERATED")) {
          console.log(`  ⚠️  ${targetFile} - missing auto-generated header`);
          manualEdits++;
          editedFiles.push(targetFile);
        } else {
          console.log(`  ✅ ${targetFile} - has auto-generated header`);
        }
      } catch (error) {
        console.log(`  ❌ ${targetFile} - file not found`);
        manualEdits++;
        editedFiles.push(targetFile);
      }
    }
  }

  console.log("\n📊 Manual Edit Detection Results:");
  console.log(`  📄 Total files: ${totalFiles}`);
  console.log(`  ✅ Auto-generated: ${totalFiles - manualEdits}`);
  console.log(`  ⚠️  Manual edits detected: ${manualEdits}`);

  if (manualEdits > 0) {
    console.log("\n⚠️  Files with potential manual edits:");
    editedFiles.forEach((file) => console.log(`  - ${file}`));

    console.log("\n💡 These files should be auto-generated. Run:");
    console.log("   node utils/distribute-from-docs.js");

    return false;
  }

  console.log("\n✅ No manual edits detected!");
  return true;
}

/**
 * Main validation function
 */
async function main() {
  const args = process.argv.slice(2);

  if (args.includes("--help") || args.includes("-h")) {
    console.log(`
Usage: node validate-sync.js [options]

Options:
  --manual-edits    Check for manual edits to generated files
  --help, -h        Show this help message

Examples:
  node validate-sync.js                 # Validate all files are in sync
  node validate-sync.js --manual-edits  # Check for manual edits
`);
    return;
  }

  if (args.includes("--manual-edits")) {
    const success = await detectManualEdits();
    if (!success) {
      process.exit(1);
    }
    return;
  }

  const success = await validateAllFiles();
  if (!success) {
    process.exit(1);
  }
}

// Only run if called directly
if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    console.error("Validation error:", error);
    process.exit(1);
  });
}

export { validateAllFiles, detectManualEdits, checkFileSync };
