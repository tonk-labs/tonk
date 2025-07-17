#!/usr/bin/env node

import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  validateSources,
  distributeSingleSource,
} from "./distribute-from-docs.js";
import { DISTRIBUTION_MAP, GENERATED_FILES } from "./file-mapping.js";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

/**
 * Test that a single source file distributes correctly
 */
async function testSingleDistribution() {
  console.log("🧪 Testing single source distribution...\n");

  // Test with components.md
  const testSource = "docs/src/llms/shared/components.md";
  console.log(`📄 Testing distribution of: ${testSource}`);

  await distributeSingleSource(testSource);

  // Verify files were created
  const targets = DISTRIBUTION_MAP[testSource];
  console.log(`\n🔍 Verifying ${targets.length} target files were created...`);

  let successCount = 0;
  let errorCount = 0;

  for (const target of targets) {
    const fullPath = path.join(__dirname, "..", target);

    try {
      const content = await fs.readFile(fullPath, "utf8");

      // Check if file has the auto-generated header
      if (content.includes("DO NOT EDIT - AUTO-GENERATED")) {
        console.log(`✅ ${target} - has auto-generated header`);
        successCount++;
      } else {
        console.log(`❌ ${target} - missing auto-generated header`);
        errorCount++;
      }

      // Check if file has actual content
      if (content.length > 200) {
        console.log(`✅ ${target} - has content (${content.length} chars)`);
      } else {
        console.log(
          `❌ ${target} - insufficient content (${content.length} chars)`
        );
        errorCount++;
      }
    } catch (error) {
      console.log(`❌ ${target} - file not found or unreadable`);
      errorCount++;
    }
  }

  console.log(`\n📊 Single distribution test results:`);
  console.log(`  ✅ Success: ${successCount}`);
  console.log(`  ❌ Errors: ${errorCount}`);

  return errorCount === 0;
}

/**
 * Test that generated files are created
 */
async function testGeneratedFiles() {
  console.log("\n🧪 Testing generated files...\n");

  // Test with first target from components
  const testSource = "docs/src/llms/shared/components.md";
  const firstTarget = DISTRIBUTION_MAP[testSource][0];
  const targetDir = path.dirname(firstTarget);

  console.log(`📄 Checking generated files in: ${targetDir}`);

  let successCount = 0;
  let errorCount = 0;

  for (const genFile of GENERATED_FILES) {
    const genPath = path.join(targetDir, genFile);
    const fullPath = path.join(__dirname, "..", genPath);

    try {
      const content = await fs.readFile(fullPath, "utf8");

      if (content.includes("DO NOT EDIT - AUTO-GENERATED")) {
        console.log(`✅ ${genPath} - exists with header`);
        successCount++;
      } else {
        console.log(`❌ ${genPath} - missing header`);
        errorCount++;
      }
    } catch (error) {
      console.log(`❌ ${genPath} - file not found`);
      errorCount++;
    }
  }

  console.log(`\n📊 Generated files test results:`);
  console.log(`  ✅ Success: ${successCount}`);
  console.log(`  ❌ Errors: ${errorCount}`);

  return errorCount === 0;
}

/**
 * Test validation functionality
 */
async function testValidation() {
  console.log("\n🧪 Testing validation...\n");

  const result = await validateSources();

  if (result) {
    console.log("✅ Validation test passed");
    return true;
  } else {
    console.log("❌ Validation test failed");
    return false;
  }
}

/**
 * Test file count expectations
 */
async function testFileCounts() {
  console.log("\n🧪 Testing file count expectations...\n");

  const sources = Object.keys(DISTRIBUTION_MAP);
  let totalTargets = 0;

  for (const targets of Object.values(DISTRIBUTION_MAP)) {
    totalTargets += targets.length;
  }

  console.log(`📚 Sources: ${sources.length}`);
  console.log(`🎯 Total targets: ${totalTargets}`);
  console.log(`📄 Generated files per target: ${GENERATED_FILES.length}`);
  console.log(
    `📄 Total files to be created: ${
      totalTargets * (1 + GENERATED_FILES.length)
    }`
  );

  // Verify mapping structure
  let mappingErrors = 0;
  for (const [source, targets] of Object.entries(DISTRIBUTION_MAP)) {
    if (!targets || targets.length === 0) {
      console.log(`❌ ${source} - no targets defined`);
      mappingErrors++;
    } else {
      console.log(`✅ ${source} - ${targets.length} targets`);
    }
  }

  console.log(`\n📊 Mapping validation results:`);
  console.log(`  ✅ Valid mappings: ${sources.length - mappingErrors}`);
  console.log(`  ❌ Invalid mappings: ${mappingErrors}`);

  return mappingErrors === 0;
}

/**
 * Clean up test files
 */
async function cleanup() {
  console.log("\n🧹 Cleaning up test files...\n");

  // Only clean up the first few files to avoid deleting everything
  const testSource = "docs/src/llms/shared/components.md";
  const testTargets = DISTRIBUTION_MAP[testSource].slice(0, 3); // Only first 3 targets

  for (const target of testTargets) {
    const fullPath = path.join(__dirname, "..", target);
    const targetDir = path.dirname(fullPath);

    try {
      // Remove main file
      await fs.unlink(fullPath);
      console.log(`🗑️  Removed: ${target}`);

      // Remove generated files
      for (const genFile of GENERATED_FILES) {
        const genPath = path.join(targetDir, genFile);
        const fullGenPath = path.join(__dirname, "..", genPath);
        try {
          await fs.unlink(fullGenPath);
          console.log(`🗑️  Removed: ${genPath}`);
        } catch (error) {
          // File might not exist, that's okay
        }
      }
    } catch (error) {
      console.log(`⚠️  Could not remove ${target}: ${error.message}`);
    }
  }

  console.log("✅ Cleanup complete");
}

/**
 * Main test function
 */
async function runTests() {
  console.log("🚀 Starting distribution tests...\n");

  const tests = [
    { name: "Validation", fn: testValidation },
    { name: "File Counts", fn: testFileCounts },
    { name: "Single Distribution", fn: testSingleDistribution },
    { name: "Generated Files", fn: testGeneratedFiles },
  ];

  let passed = 0;
  let failed = 0;

  for (const test of tests) {
    try {
      const result = await test.fn();
      if (result) {
        console.log(`✅ ${test.name} test PASSED\n`);
        passed++;
      } else {
        console.log(`❌ ${test.name} test FAILED\n`);
        failed++;
      }
    } catch (error) {
      console.log(`❌ ${test.name} test ERROR: ${error.message}\n`);
      failed++;
    }
  }

  console.log("📊 Test Summary:");
  console.log(`  ✅ Passed: ${passed}`);
  console.log(`  ❌ Failed: ${failed}`);
  console.log(`  📊 Total: ${passed + failed}`);

  // Ask if user wants to cleanup
  if (passed > 0) {
    console.log(
      "\n🧹 Test created some files. Run with --cleanup to remove them."
    );
  }

  return failed === 0;
}

/**
 * CLI interface
 */
async function main() {
  const args = process.argv.slice(2);

  if (args.includes("--help") || args.includes("-h")) {
    console.log(`
Usage: node test-distribution.js [options]

Options:
  --cleanup     Remove test files after testing
  --help, -h    Show this help message

Examples:
  node test-distribution.js           # Run all tests
  node test-distribution.js --cleanup # Run tests and cleanup
`);
    return;
  }

  if (args.includes("--cleanup")) {
    await cleanup();
    return;
  }

  const success = await runTests();

  if (!success) {
    process.exit(1);
  }
}

// Only run if called directly
if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    console.error("Test runner error:", error);
    process.exit(1);
  });
}

export {
  runTests,
  testSingleDistribution,
  testGeneratedFiles,
  testValidation,
  cleanup,
};
