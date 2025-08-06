#!/usr/bin/env node

/**
 * Bundle Analyzer Script
 *
 * This script demonstrates the comprehensive capabilities of the @tonk/spec library
 * by creating, packing, unpacking, and analyzing bundles with detailed information output.
 *
 * Run with: npx tsx examples/bundle-analyzer.ts
 */

import { promises as fs } from 'fs';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';
import { existsSync } from 'fs';

// Import the bundle library
import {
  ZipBundle,
  createEmptyBundle,
  createBundleFromFiles,
  getBundleInfo,
  formatBytes,
  guessMimeType,
  parseBundle,
  validateBundle,
  validateBundleComprehensive,
  formatValidationErrors,
  generateValidationReport,
  detectCircularEntrypointReferences,
  type Bundle,
  type BundleFile,
  type BundleInfo,
  type DetailedValidationResult,
} from '../src/index.js';

// Get the current directory
const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

/**
 * Color codes for console output
 */
const colors = {
  reset: '\x1b[0m',
  bright: '\x1b[1m',
  red: '\x1b[31m',
  green: '\x1b[32m',
  yellow: '\x1b[33m',
  blue: '\x1b[34m',
  magenta: '\x1b[35m',
  cyan: '\x1b[36m',
  white: '\x1b[37m',
};

/**
 * Utility function to log with colors
 */
function log(message: string, color: keyof typeof colors = 'reset') {
  console.log(`${colors[color]}${message}${colors.reset}`);
}

/**
 * Print a section header
 */
function printHeader(title: string) {
  const border = '='.repeat(80);
  log('\n' + border, 'cyan');
  log(`  ${title}`, 'bright');
  log(border, 'cyan');
}

/**
 * Print a subsection header
 */
function printSubHeader(title: string) {
  log(`\n--- ${title} ---`, 'yellow');
}

/**
 * Create sample files for demonstration
 */
function createSampleFiles(): Map<string, ArrayBuffer> {
  const files = new Map<string, ArrayBuffer>();

  // HTML file
  const htmlContent = `<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Bundle Demo App</title>
    <link rel="stylesheet" href="/styles/main.css">
</head>
<body>
    <div class="container">
        <h1>Welcome to Bundle Demo</h1>
        <p>This is a sample application packed in a bundle!</p>
        <button id="demo-btn">Click me!</button>
    </div>
    <script src="/scripts/app.js"></script>
</body>
</html>`;

  // CSS file
  const cssContent = `/* Main stylesheet */
.container {
    max-width: 800px;
    margin: 0 auto;
    padding: 20px;
    font-family: Arial, sans-serif;
}

h1 {
    color: #333;
    text-align: center;
    margin-bottom: 20px;
}

p {
    font-size: 16px;
    line-height: 1.6;
    color: #666;
}

button {
    background-color: #007bff;
    color: white;
    border: none;
    padding: 10px 20px;
    border-radius: 5px;
    cursor: pointer;
    font-size: 16px;
}

button:hover {
    background-color: #0056b3;
}

.highlight {
    background-color: #fff3cd;
    border: 1px solid #ffeaa7;
    padding: 10px;
    border-radius: 4px;
    margin: 10px 0;
}`;

  // JavaScript file
  const jsContent = `// Main application script
document.addEventListener('DOMContentLoaded', function() {
    console.log('Bundle Demo App loaded successfully!');
    
    const button = document.getElementById('demo-btn');
    if (button) {
        button.addEventListener('click', function() {
            alert('Hello from the bundled app!');
            
            // Create a highlight div
            const highlight = document.createElement('div');
            highlight.className = 'highlight';
            highlight.textContent = 'Button clicked! Bundle is working correctly.';
            document.querySelector('.container').appendChild(highlight);
        });
    }
    
    // Demonstrate some more complex functionality
    fetch('/api/data.json')
        .then(response => response.json())
        .then(data => {
            console.log('Data loaded:', data);
        })
        .catch(error => {
            console.log('Mock API call - this is expected in the demo');
        });
});`;

  // JSON data file
  const jsonContent = `{
    "name": "Bundle Demo Data",
    "version": "1.0.0",
    "features": [
        "HTML templating",
        "CSS styling",
        "JavaScript interactivity",
        "JSON data handling"
    ],
    "metadata": {
        "created": "${new Date().toISOString()}",
        "bundleType": "demo",
        "fileCount": 4
    }
}`;

  // README file
  const readmeContent = `# Bundle Demo Application

This is a sample application demonstrating the capabilities of the Tonk bundle format.

## Contents

- **index.html**: Main HTML page
- **styles/main.css**: Stylesheet for the application
- **scripts/app.js**: JavaScript functionality
- **data/demo.json**: Sample JSON data

## Features

- Responsive design
- Interactive button
- Data fetching simulation
- Modern ES6+ JavaScript

## Bundle Information

This bundle was created using the @tonk/spec library and demonstrates:
- File organization with virtual paths
- Multiple file types and MIME type detection
- Entrypoint configuration
- Bundle validation and integrity checking
`;

  // Convert strings to ArrayBuffers
  files.set('/index.html', new TextEncoder().encode(htmlContent).buffer);
  files.set('/styles/main.css', new TextEncoder().encode(cssContent).buffer);
  files.set('/scripts/app.js', new TextEncoder().encode(jsContent).buffer);
  files.set('/data/demo.json', new TextEncoder().encode(jsonContent).buffer);
  files.set('/README.md', new TextEncoder().encode(readmeContent).buffer);

  return files;
}

/**
 * Display detailed bundle information
 */
function displayBundleInfo(bundle: Bundle) {
  const info = getBundleInfo(bundle);
  const manifest = bundle.manifest;

  printSubHeader('Bundle Overview');
  console.log(`📦 Version: ${manifest.version}`);
  console.log(`📅 Created: ${manifest.createdAt || 'Unknown'}`);
  console.log(`📁 Total Files: ${info.fileCount}`);
  console.log(`📊 Total Size: ${formatBytes(info.totalSize)}`);
  // Note: Compression information would be available after serialization
  console.log(
    `📊 Estimated compressed size: ${formatBytes(info.totalSize * 0.7)}`
  ); // Rough estimate

  // Display entrypoints
  printSubHeader('Entrypoints');
  const entrypoints = Object.entries(manifest.entrypoints);
  if (entrypoints.length > 0) {
    entrypoints.forEach(([name, path]) => {
      console.log(`🎯 ${name}: ${path}`);
    });
  } else {
    console.log('🔹 No entrypoints defined');
  }

  // Display metadata if present
  if (manifest.metadata && Object.keys(manifest.metadata).length > 0) {
    printSubHeader('Bundle Metadata');
    Object.entries(manifest.metadata).forEach(([key, value]) => {
      console.log(`🏷️  ${key}: ${JSON.stringify(value)}`);
    });
  }
}

/**
 * Display detailed file information
 */
async function displayFileDetails(bundle: Bundle) {
  printSubHeader('File Analysis');

  const files = bundle.listFiles();
  const filesByType = new Map<string, BundleFile[]>();

  // Group files by MIME type
  files.forEach(file => {
    const type = file.contentType;
    if (!filesByType.has(type)) {
      filesByType.set(type, []);
    }
    filesByType.get(type)!.push(file);
  });

  // Display summary by type
  console.log('\n📋 Files by Type:');
  for (const [mimeType, typeFiles] of filesByType) {
    console.log(`   ${mimeType}: ${typeFiles.length} file(s)`);
  }

  console.log('\n📄 Individual Files:');
  for (const file of files) {
    console.log(`\n   📁 ${file.path}`);
    console.log(`      💾 Size: ${formatBytes(file.uncompressedSize || 0)}`); // Use uncompressedSize
    console.log(`      🏷️  MIME: ${file.contentType}`);

    if (file.lastModified) {
      console.log(`      📅 Modified: ${file.lastModified}`);
    }

    // Show content preview for text files
    if (
      file.contentType.startsWith('text/') ||
      file.contentType === 'application/json' ||
      file.path.endsWith('.md')
    ) {
      try {
        const data = await bundle.getFileData(file.path);
        if (data) {
          const text = new TextDecoder().decode(data);
          const preview =
            text.length > 200 ? text.substring(0, 200) + '...' : text;
          console.log(`      👁️  Preview: ${preview.replace(/\n/g, '\\n')}`);
        }
      } catch (error) {
        console.log(`      ❌ Error reading file: ${error}`);
      }
    }
  }
}

/**
 * Perform comprehensive bundle validation
 */
async function validateBundleDetails(bundle: Bundle) {
  printSubHeader('Bundle Validation');

  try {
    // Basic validation
    const basicValidation = bundle.validate();
    console.log(
      `✅ Basic validation: ${basicValidation.valid ? 'PASSED' : 'FAILED'}`
    );

    if (!basicValidation.valid) {
      console.log('🔍 Basic validation errors:');
      basicValidation.errors.forEach(error => {
        log(`   ❌ ${error.message}`, 'red');
      });
    }

    // Comprehensive validation with detailed options
    const comprehensiveValidation = bundle.validate({
      includeWarnings: true,
      includeInfo: true,
      maxBundleSize: 10 * 1024 * 1024, // 10MB
    });
    console.log(
      `✅ Comprehensive validation: ${comprehensiveValidation.valid ? 'PASSED' : 'FAILED'}`
    );

    if (!comprehensiveValidation.valid) {
      console.log('🔍 Comprehensive validation errors:');
      comprehensiveValidation.errors.forEach(error => {
        log(`   ❌ ${error.message}`, 'red');
      });
    }

    if (
      comprehensiveValidation.warnings &&
      comprehensiveValidation.warnings.length > 0
    ) {
      console.log('🔍 Warnings:');
      comprehensiveValidation.warnings.forEach(warning => {
        log(`   ⚠️ ${warning.message}`, 'yellow');
      });
    }

    // Check for circular entrypoint references
    console.log('\n🔄 Checking for circular entrypoint references...');
    const circularRefs = detectCircularEntrypointReferences(
      bundle.manifest.entrypoints
    );
    if (circularRefs.length > 0) {
      log('⚠️ Circular references detected:', 'yellow');
      circularRefs.forEach(ref => {
        log(`   🔄 ${ref}`, 'yellow');
      });
    } else {
      log('✅ No circular references found', 'green');
    }

    // Generate validation report
    try {
      const report = generateValidationReport(comprehensiveValidation);
      if (report.length > 0) {
        console.log('\n📊 Validation Report:');
        console.log(report);
      }
    } catch (error) {
      console.log('ℹ️ Validation report generation not available');
    }
  } catch (error) {
    log(`❌ Validation failed: ${error}`, 'red');
  }
}

/**
 * Demonstrate bundle serialization and size analysis
 */
async function analyzeBundlePerformance(bundle: Bundle) {
  printSubHeader('Performance Analysis');

  try {
    // Measure serialization time
    const startTime = performance.now();
    const bundleData = await bundle.toArrayBuffer();
    const serializationTime = performance.now() - startTime;

    console.log(`⏱️  Serialization time: ${serializationTime.toFixed(2)}ms`);
    console.log(`📦 Serialized size: ${formatBytes(bundleData.byteLength)}`);

    // Estimate bundle size
    const estimatedSize = bundle.estimateBundleSize();
    console.log(`📏 Estimated size: ${formatBytes(estimatedSize)}`);

    const accuracy =
      (Math.abs(bundleData.byteLength - estimatedSize) /
        bundleData.byteLength) *
      100;
    console.log(`🎯 Size estimation accuracy: ${(100 - accuracy).toFixed(1)}%`);

    // Calculate files per size bracket
    const files = bundle.listFiles();
    const sizeBrackets = {
      'tiny (< 1KB)': 0,
      'small (1KB - 10KB)': 0,
      'medium (10KB - 100KB)': 0,
      'large (> 100KB)': 0,
    };

    files.forEach(file => {
      const size = file.uncompressedSize || 0;
      if (size < 1024) sizeBrackets['tiny (< 1KB)']++;
      else if (size < 10240) sizeBrackets['small (1KB - 10KB)']++;
      else if (size < 102400) sizeBrackets['medium (10KB - 100KB)']++;
      else sizeBrackets['large (> 100KB)']++;
    });

    console.log('\n📊 File size distribution:');
    Object.entries(sizeBrackets).forEach(([bracket, count]) => {
      if (count > 0) {
        console.log(`   ${bracket}: ${count} file(s)`);
      }
    });

    return bundleData;
  } catch (error) {
    log(`❌ Performance analysis failed: ${error}`, 'red');
    return null;
  }
}

/**
 * Demonstrate bundle unpacking and parsing
 */
async function demonstrateUnpacking(bundleData: ArrayBuffer) {
  printSubHeader('Bundle Unpacking & Parsing');

  try {
    console.log('📂 Parsing bundle from ArrayBuffer...');
    const startTime = performance.now();

    // Parse with strict validation
    const parsedBundle = await parseBundle(bundleData, {
      strictValidation: true,
      validateFileReferences: true,
      maxSize: 10 * 1024 * 1024, // 10MB limit
    });

    const parseTime = performance.now() - startTime;
    log(`✅ Bundle parsed successfully in ${parseTime.toFixed(2)}ms`, 'green');

    // Verify the parsed bundle matches the original
    console.log('\n🔍 Verifying bundle integrity...');
    const originalFileCount = parsedBundle.getFileCount();
    const originalInfo = getBundleInfo(parsedBundle);

    console.log(`📁 Parsed file count: ${originalFileCount}`);
    console.log(`📊 Parsed total size: ${formatBytes(originalInfo.totalSize)}`);

    // Test file access
    console.log('\n🧪 Testing file access...');
    const testPath = '/index.html';
    if (parsedBundle.hasFile(testPath)) {
      const fileData = await parsedBundle.getFileData(testPath);
      if (fileData) {
        const content = new TextDecoder().decode(fileData);
        log(
          `✅ Successfully read ${testPath} (${fileData.byteLength} bytes)`,
          'green'
        );
        console.log(`   Preview: ${content.substring(0, 100)}...`);
      }
    }

    return parsedBundle;
  } catch (error) {
    log(`❌ Bundle unpacking failed: ${error}`, 'red');
    return null;
  }
}

/**
 * Compare two bundles for differences
 */
function compareBundles(original: Bundle, parsed: Bundle) {
  printSubHeader('Bundle Comparison');

  const originalFiles = original.listFiles();
  const parsedFiles = parsed.listFiles();

  console.log(`📊 Original bundle: ${originalFiles.length} files`);
  console.log(`📊 Parsed bundle: ${parsedFiles.length} files`);

  if (originalFiles.length === parsedFiles.length) {
    log('✅ File counts match', 'green');
  } else {
    log('❌ File counts do not match', 'red');
  }

  // Check file paths
  const originalPaths = new Set(originalFiles.map(f => f.path));
  const parsedPaths = new Set(parsedFiles.map(f => f.path));

  const missing = [...originalPaths].filter(path => !parsedPaths.has(path));
  const extra = [...parsedPaths].filter(path => !originalPaths.has(path));

  if (missing.length === 0 && extra.length === 0) {
    log('✅ All file paths match', 'green');
  } else {
    if (missing.length > 0) {
      log(`❌ Missing files: ${missing.join(', ')}`, 'red');
    }
    if (extra.length > 0) {
      log(`❌ Extra files: ${extra.join(', ')}`, 'red');
    }
  }

  // Compare manifests
  const originalManifest = original.manifest;
  const parsedManifest = parsed.manifest;

  if (originalManifest.version === parsedManifest.version) {
    log('✅ Bundle versions match', 'green');
  } else {
    log(
      `❌ Version mismatch: ${originalManifest.version} vs ${parsedManifest.version}`,
      'red'
    );
  }

  const originalEntrypoints = Object.keys(originalManifest.entrypoints).length;
  const parsedEntrypoints = Object.keys(parsedManifest.entrypoints).length;

  if (originalEntrypoints === parsedEntrypoints) {
    log('✅ Entrypoint counts match', 'green');
  } else {
    log(
      `❌ Entrypoint count mismatch: ${originalEntrypoints} vs ${parsedEntrypoints}`,
      'red'
    );
  }
}

/**
 * Extract bundle contents to a directory for inspection
 */
async function extractBundleContents(bundle: Bundle, outputDir: string) {
  printSubHeader('Bundle Content Extraction');

  // Create output directory
  await fs.mkdir(outputDir, { recursive: true });
  console.log(`📁 Creating extraction directory: ${outputDir}`);

  const files = bundle.listFiles();
  let extractedCount = 0;

  for (const file of files) {
    try {
      const filePath = join(outputDir, file.path.substring(1)); // Remove leading slash
      const fileDir = dirname(filePath);

      // Create directory structure
      await fs.mkdir(fileDir, { recursive: true });

      // Extract file content
      const fileData = await bundle.getFileData(file.path);
      if (fileData) {
        await fs.writeFile(filePath, Buffer.from(fileData));
        extractedCount++;
        console.log(`   📄 Extracted: ${file.path} -> ${filePath}`);
      }
    } catch (error) {
      log(`   ❌ Failed to extract ${file.path}: ${error}`, 'red');
    }
  }

  log(`✅ Extracted ${extractedCount}/${files.length} files`, 'green');
  return extractedCount;
}

/**
 * Create an HTML viewer for the bundle contents
 */
async function createBundleViewer(bundle: Bundle, outputDir: string) {
  printSubHeader('Creating Bundle Viewer');

  const manifest = bundle.manifest;
  const info = getBundleInfo(bundle);
  const files = bundle.listFiles();

  // Create HTML viewer
  const viewerHtml = `<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Bundle Inspector - ${manifest.version ? `v${manifest.version}` : 'Unknown Version'}</title>
    <style>
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            line-height: 1.6;
            margin: 0;
            padding: 20px;
            background-color: #f5f5f5;
        }
        .container {
            max-width: 1200px;
            margin: 0 auto;
            background: white;
            border-radius: 8px;
            box-shadow: 0 2px 10px rgba(0,0,0,0.1);
            overflow: hidden;
        }
        .header {
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            padding: 30px;
            text-align: center;
        }
        .header h1 {
            margin: 0;
            font-size: 2.5em;
        }
        .header p {
            margin: 10px 0 0 0;
            opacity: 0.9;
        }
        .content {
            padding: 30px;
        }
        .section {
            margin-bottom: 40px;
        }
        .section h2 {
            color: #333;
            border-bottom: 2px solid #eee;
            padding-bottom: 10px;
        }
        .info-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
            gap: 20px;
            margin-bottom: 30px;
        }
        .info-card {
            background: #f8f9fa;
            padding: 20px;
            border-radius: 6px;
            border-left: 4px solid #667eea;
        }
        .info-card h3 {
            margin: 0 0 10px 0;
            color: #333;
        }
        .info-card p {
            margin: 0;
            font-size: 1.1em;
            color: #666;
        }
        .file-list {
            display: grid;
            gap: 15px;
        }
        .file-item {
            background: #f8f9fa;
            padding: 15px;
            border-radius: 6px;
            border: 1px solid #e9ecef;
            transition: all 0.2s ease;
        }
        .file-item:hover {
            background: #e9ecef;
            transform: translateY(-1px);
            box-shadow: 0 2px 8px rgba(0,0,0,0.1);
        }
        .file-header {
            display: flex;
            justify-content: space-between;
            align-items: center;
            margin-bottom: 10px;
        }
        .file-path {
            font-family: 'Courier New', monospace;
            font-weight: bold;
            color: #333;
        }
        .file-size {
            background: #667eea;
            color: white;
            padding: 4px 8px;
            border-radius: 4px;
            font-size: 0.9em;
        }
        .file-meta {
            display: flex;
            gap: 15px;
            font-size: 0.9em;
            color: #666;
            margin-bottom: 10px;
        }
        .file-preview {
            background: #fff;
            border: 1px solid #ddd;
            border-radius: 4px;
            padding: 10px;
            font-family: 'Courier New', monospace;
            font-size: 0.9em;
            max-height: 200px;
            overflow-y: auto;
            white-space: pre-wrap;
            word-break: break-all;
        }
        .entrypoint-badge {
            background: #28a745;
            color: white;
            padding: 2px 8px;
            border-radius: 12px;
            font-size: 0.8em;
            margin-left: 10px;
        }
        .mime-type {
            background: #17a2b8;
            color: white;
            padding: 2px 8px;
            border-radius: 4px;
            font-size: 0.8em;
        }
        .stats-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
            gap: 15px;
        }
        .stat-item {
            text-align: center;
            padding: 15px;
            background: #f8f9fa;
            border-radius: 6px;
        }
        .stat-number {
            font-size: 2em;
            font-weight: bold;
            color: #667eea;
            display: block;
        }
        .nav {
            background: #333;
            padding: 15px 30px;
        }
        .nav a {
            color: #fff;
            text-decoration: none;
            margin-right: 20px;
            padding: 8px 16px;
            border-radius: 4px;
            transition: background 0.2s;
        }
        .nav a:hover {
            background: #555;
        }
        .metadata {
            background: #f8f9fa;
            padding: 15px;
            border-radius: 6px;
            border: 1px solid #e9ecef;
        }
        .metadata pre {
            margin: 0;
            background: #fff;
            padding: 10px;
            border-radius: 4px;
            border: 1px solid #ddd;
            overflow-x: auto;
        }
    </style>
</head>
<body>
    <div class="container">
        <div class="header">
            <h1>📦 Bundle Inspector</h1>
            <p>Interactive view of bundle contents and metadata</p>
        </div>
        
        <div class="nav">
            <a href="#overview">Overview</a>
            <a href="#files">Files</a>
            <a href="#entrypoints">Entrypoints</a>
            <a href="#metadata">Metadata</a>
        </div>

        <div class="content">
            <section id="overview" class="section">
                <h2>📊 Bundle Overview</h2>
                <div class="info-grid">
                    <div class="info-card">
                        <h3>Version</h3>
                        <p>${manifest.version || 'Unknown'}</p>
                    </div>
                    <div class="info-card">
                        <h3>Created</h3>
                        <p>${manifest.createdAt ? new Date(manifest.createdAt).toLocaleString() : 'Unknown'}</p>
                    </div>
                    <div class="info-card">
                        <h3>File Count</h3>
                        <p>${info.fileCount} files</p>
                    </div>
                    <div class="info-card">
                        <h3>Total Size</h3>
                        <p>${formatBytes(info.totalSize)}</p>
                    </div>
                </div>
            </section>

            <section id="entrypoints" class="section">
                <h2>🎯 Entrypoints</h2>
                ${
                  Object.keys(manifest.entrypoints).length > 0
                    ? `
                    <div class="stats-grid">
                        ${Object.entries(manifest.entrypoints)
                          .map(
                            ([name, path]) => `
                            <div class="stat-item">
                                <span class="stat-number">${name}</span>
                                <div style="font-family: monospace; color: #666; margin-top: 5px;">${path}</div>
                            </div>
                        `
                          )
                          .join('')}
                    </div>
                `
                    : '<p>No entrypoints defined</p>'
                }
            </section>

            <section id="files" class="section">
                <h2>📁 Files (${files.length})</h2>
                <div class="file-list">
                    ${await Promise.all(
                      files.map(async file => {
                        const isEntrypoint = Object.values(
                          manifest.entrypoints
                        ).includes(file.path);
                        const entrypointNames = Object.entries(
                          manifest.entrypoints
                        )
                          .filter(([, path]) => path === file.path)
                          .map(([name]) => name);

                        let preview = '';
                        try {
                          const fileData = await bundle.getFileData(file.path);
                          if (
                            fileData &&
                            (file.contentType.startsWith('text/') ||
                              file.contentType === 'application/json' ||
                              file.path.endsWith('.md'))
                          ) {
                            const content = new TextDecoder().decode(fileData);
                            preview =
                              content.length > 500
                                ? content.substring(0, 500) + '...'
                                : content;
                          } else if (fileData) {
                            preview = `Binary file (${formatBytes(fileData.byteLength)})`;
                          }
                        } catch (error) {
                          preview = `Error reading file: ${error}`;
                        }

                        return `
                            <div class="file-item">
                                <div class="file-header">
                                    <span class="file-path">${file.path}</span>
                                    <span class="file-size">${formatBytes(file.uncompressedSize || 0)}</span>
                                </div>
                                <div class="file-meta">
                                    <span class="mime-type">${file.contentType}</span>
                                    ${isEntrypoint ? entrypointNames.map(name => `<span class="entrypoint-badge">📍 ${name}</span>`).join('') : ''}
                                    ${file.lastModified ? `<span>Modified: ${new Date(file.lastModified).toLocaleString()}</span>` : ''}
                                </div>
                                ${preview ? `<div class="file-preview">${preview.replace(/</g, '&lt;').replace(/>/g, '&gt;')}</div>` : ''}
                            </div>
                        `;
                      })
                    )}
                </div>
            </section>

            <section id="metadata" class="section">
                <h2>📋 Bundle Metadata</h2>
                <div class="metadata">
                    <h3>Manifest</h3>
                    <pre>${JSON.stringify(manifest, null, 2)}</pre>
                </div>
                
                <div class="metadata" style="margin-top: 20px;">
                    <h3>Bundle Information</h3>
                    <pre>${JSON.stringify(info, null, 2)}</pre>
                </div>
            </section>
        </div>
    </div>

    <script>
        // Smooth scrolling for navigation
        document.querySelectorAll('.nav a').forEach(link => {
            link.addEventListener('click', (e) => {
                e.preventDefault();
                const target = document.querySelector(link.getAttribute('href'));
                if (target) {
                    target.scrollIntoView({ behavior: 'smooth' });
                }
            });
        });
    </script>
</body>
</html>`;

  const viewerPath = join(outputDir, 'bundle-viewer.html');
  await fs.writeFile(viewerPath, viewerHtml);

  log(`✅ Created bundle viewer: ${viewerPath}`, 'green');
  log(`🌐 Open in browser: file://${viewerPath}`, 'cyan');

  return viewerPath;
}

/**
 * Main execution function
 */
async function main() {
  // Parse command line arguments
  const args = process.argv.slice(2);
  const shouldExtract = args.includes('--extract') || args.includes('-e');
  const shouldCreateViewer = args.includes('--viewer') || args.includes('-v');
  const shouldKeepFiles = args.includes('--keep') || args.includes('-k');
  const outputDir =
    args.find(arg => arg.startsWith('--output='))?.split('=')[1] ||
    join(__dirname, 'bundle-output');

  log('🚀 Starting Bundle Analyzer Demo', 'bright');
  log(
    'This script demonstrates the comprehensive capabilities of @tonk/spec',
    'cyan'
  );

  if (shouldExtract || shouldCreateViewer || shouldKeepFiles) {
    console.log('\n🔧 Options:');
    if (shouldExtract) console.log('   📁 Will extract bundle contents');
    if (shouldCreateViewer) console.log('   🌐 Will create HTML viewer');
    if (shouldKeepFiles) console.log('   💾 Will keep all generated files');
    console.log(`   📂 Output directory: ${outputDir}`);
  }

  try {
    // Phase 1: Create a bundle from sample files
    printHeader('Phase 1: Bundle Creation');

    console.log('📝 Creating sample files...');
    const sampleFiles = createSampleFiles();
    console.log(`✅ Created ${sampleFiles.size} sample files`);

    console.log('\n📦 Creating bundle from files...');
    const bundle = await createBundleFromFiles(sampleFiles, {
      autoDetectTypes: true,
    });

    // Set up entrypoints
    bundle.setEntrypoint('main', '/index.html');
    bundle.setEntrypoint('styles', '/styles/main.css');
    bundle.setEntrypoint('script', '/scripts/app.js');

    log('✅ Bundle created successfully', 'green');

    // Phase 2: Analyze the created bundle
    printHeader('Phase 2: Bundle Analysis');
    displayBundleInfo(bundle);
    await displayFileDetails(bundle);
    await validateBundleDetails(bundle);

    // Phase 3: Performance analysis and serialization
    printHeader('Phase 3: Performance Analysis & Serialization');
    const bundleData = await analyzeBundlePerformance(bundle);

    if (!bundleData) {
      throw new Error('Failed to serialize bundle');
    }

    // Phase 4: Save and demonstrate unpacking
    printHeader('Phase 4: Bundle Unpacking & Verification');

    // Save bundle file
    const bundlePath = join(outputDir, 'demo-bundle.zip');
    await fs.mkdir(outputDir, { recursive: true });
    await fs.writeFile(bundlePath, Buffer.from(bundleData));
    console.log(`💾 Bundle saved to: ${bundlePath}`);
    console.log(`📁 File size: ${formatBytes(bundleData.byteLength)}`);

    // Demonstrate unpacking
    const parsedBundle = await demonstrateUnpacking(bundleData);

    if (parsedBundle) {
      // Phase 5: Compare original and parsed bundles
      printHeader('Phase 5: Bundle Integrity Verification');
      compareBundles(bundle, parsedBundle);

      // Phase 6: Extract bundle contents and create viewer if requested
      let extractedFiles = 0;
      let viewerPath = '';

      if (shouldExtract || shouldCreateViewer) {
        printHeader('Phase 6: Bundle Content Extraction & Viewer Creation');

        if (shouldExtract) {
          const extractDir = join(outputDir, 'extracted');
          extractedFiles = await extractBundleContents(
            parsedBundle,
            extractDir
          );
        }

        if (shouldCreateViewer) {
          viewerPath = await createBundleViewer(parsedBundle, outputDir);
        }
      }

      // Enhanced final summary
      printHeader('Analysis Complete');
      log('🎉 Bundle analysis completed successfully!', 'green');
      log(`📊 Summary:`, 'bright');
      console.log(`   • Created bundle with ${bundle.getFileCount()} files`);
      console.log(
        `   • Total size: ${formatBytes(getBundleInfo(bundle).totalSize)}`
      );
      console.log(
        `   • Serialized size: ${formatBytes(bundleData.byteLength)}`
      );
      console.log(
        `   • Validation: ${bundle.validate().valid ? 'PASSED' : 'FAILED'}`
      );
      console.log(`   • Bundle file: ${bundlePath}`);

      if (extractedFiles > 0) {
        console.log(
          `   • Extracted ${extractedFiles} files to: ${join(outputDir, 'extracted')}`
        );
      }

      if (viewerPath) {
        console.log(`   • Interactive viewer: ${viewerPath}`);
        log(
          `\n🌐 To inspect the bundle visually, open: file://${viewerPath}`,
          'cyan'
        );
      }

      if (!shouldKeepFiles) {
        // Clean up bundle file after a delay
        setTimeout(async () => {
          try {
            if (!shouldExtract && !shouldCreateViewer) {
              await fs.unlink(bundlePath);
              console.log('🧹 Temporary bundle file cleaned up');
            }
          } catch (error) {
            // Ignore cleanup errors
          }
        }, 10000);
      } else {
        log(`\n💾 All files preserved in: ${outputDir}`, 'green');
      }
    }
  } catch (error) {
    log(`❌ Analysis failed: ${error}`, 'red');
    if (error instanceof Error) {
      console.error(error.stack);
    }
    process.exit(1);
  }
}

/**
 * Show usage help
 */
function showHelp() {
  console.log(`
📦 Bundle Analyzer - Visual Bundle Inspection Tool

Usage: npx tsx bundle-analyzer.ts [options]

Options:
  -e, --extract     Extract bundle contents to files
  -v, --viewer      Create interactive HTML viewer
  -k, --keep        Keep all generated files (don't auto-cleanup)
  --output=DIR      Set output directory (default: ./bundle-output)
  -h, --help        Show this help message

Examples:
  npx tsx bundle-analyzer.ts                    # Basic analysis only
  npx tsx bundle-analyzer.ts --viewer           # Create HTML viewer
  npx tsx bundle-analyzer.ts --extract          # Extract all files
  npx tsx bundle-analyzer.ts -v -e -k           # Full analysis + keep files
  npx tsx bundle-analyzer.ts --output=./my-bundle --viewer

The HTML viewer provides:
  • Interactive file browser with content previews
  • Bundle metadata and statistics
  • File type categorization
  • Entrypoint visualization
  • Responsive design for easy inspection

Generated files:
  • demo-bundle.zip      - The packed bundle
  • bundle-viewer.html   - Interactive HTML viewer
  • extracted/           - Extracted bundle contents (if --extract)
`);
}

// Execute if run directly
if (import.meta.url === `file://${process.argv[1]}`) {
  const args = process.argv.slice(2);

  if (args.includes('--help') || args.includes('-h')) {
    showHelp();
    process.exit(0);
  }

  main().catch(error => {
    console.error('Fatal error:', error);
    process.exit(1);
  });
}

export { main };
