# Bundle Analyzer Examples

This directory contains example scripts that demonstrate the comprehensive capabilities of the
`@tonk/spec` bundle library, including **visual bundle inspection** tools.

## Scripts

### 1. Bundle Analyzer (`bundle-analyzer.ts`)

A comprehensive demonstration script that showcases all major features of the bundle library:

- **Bundle Creation**: Creates a sample web application bundle with HTML, CSS, JavaScript, JSON, and
  Markdown files
- **Detailed Analysis**: Provides comprehensive information about bundle contents, metadata, and
  structure
- **File Inspection**: Shows file-by-file analysis with content previews and metadata
- **Validation**: Performs both basic and comprehensive bundle validation
- **Performance Metrics**: Measures serialization time, compression ratios, and size analysis
- **Pack/Unpack Cycle**: Demonstrates complete bundle serialization and parsing cycle
- **Integrity Verification**: Compares original and unpacked bundles for consistency
- **🌐 Visual Inspection**: Creates interactive HTML viewer for browsing bundle contents
- **📁 File Extraction**: Extracts all bundle files to inspectable directory structure

### 2. Simple Example (`simple-example.ts`)

A shorter, focused example that demonstrates basic bundle operations:

- Basic bundle creation with sample files
- File listing and metadata display
- Pack and unpack operations
- Content verification

## Running the Scripts

### Prerequisites

Make sure you have built the @tonk/spec library:

```bash
cd packages/spec
npm run build
```

### Quick Start - Visual Bundle Inspection

For the full visual inspection experience:

```bash
cd packages/spec/examples
npm install
npm run analyze
```

This creates an interactive HTML viewer that opens in your browser.

### Other Options

```bash
# Basic analysis only (no files saved)
npm run analyze:basic

# Extract all files + create viewer
npm run analyze:extract

# Just create the HTML viewer
npm run viewer

# Simple example
npm run analyze:simple
```

### Direct Usage

```bash
cd packages/spec

# Create interactive HTML viewer (recommended)
npx tsx examples/bundle-analyzer.ts --viewer --keep

# Extract all files to directory
npx tsx examples/bundle-analyzer.ts --extract

# Full analysis with all options
npx tsx examples/bundle-analyzer.ts --extract --viewer --keep

# Basic analysis only
npx tsx examples/bundle-analyzer.ts

# Show help
npx tsx examples/bundle-analyzer.ts --help
```

### Run the simple example

```bash
cd packages/spec/examples
npm run analyze:simple
```

Or directly:

```bash
cd packages/spec
npx tsx examples/simple-example.ts
```

## Example Output

The comprehensive analyzer produces detailed output including:

### Bundle Overview

- Bundle version and creation timestamp
- Total file count and sizes
- Compression ratios
- Entrypoint definitions
- Bundle metadata

### File Analysis

- Files grouped by MIME type
- Individual file details with sizes and content previews
- Content type detection results

### Validation Results

- Basic bundle structure validation
- Comprehensive validation with detailed error reporting
- Circular reference detection
- Validation reports

### Performance Metrics

- Serialization timing
- Size estimation accuracy
- File size distribution analysis
- Compression effectiveness

### Pack/Unpack Verification

- Round-trip integrity testing
- File count and path verification
- Content consistency checks
- Bundle comparison results

## Key Features Demonstrated

### Bundle Creation

```typescript
import { createBundleFromFiles, ZipBundle } from '@tonk/spec';

const files = new Map([
  ['/index.html', htmlBuffer],
  ['/style.css', cssBuffer],
  ['/app.js', jsBuffer],
]);

const bundle = await createBundleFromFiles(files, {
  autoDetectTypes: true,
});
```

### Bundle Analysis

```typescript
import { getBundleInfo, formatBytes } from '@tonk/spec';

const info = getBundleInfo(bundle);
console.log(`Files: ${info.fileCount}`);
console.log(`Size: ${formatBytes(info.totalSize)}`);
```

### Pack and Unpack

```typescript
import { parseBundle } from '@tonk/spec';

// Pack
const packed = await bundle.toArrayBuffer();

// Unpack
const unpacked = await parseBundle(packed);
```

### Validation

```typescript
import { validateBundleComprehensive } from '@tonk/spec';

const validation = await validateBundleComprehensive(bundle);
console.log(`Valid: ${validation.valid}`);
```

## File Structure

The examples create bundles containing:

- **HTML**: Main application page with proper structure
- **CSS**: Styled components with modern CSS
- **JavaScript**: Interactive functionality with DOM manipulation
- **JSON**: Configuration and data files
- **Markdown**: Documentation and README files

All files include realistic content that demonstrates proper file relationships and dependencies
that would be found in real-world applications.

## Error Handling

The scripts include comprehensive error handling and will display:

- Validation errors with severity levels
- Parsing failures with detailed messages
- File access errors
- Performance metrics even when operations partially fail

## Customization

You can modify the scripts to:

- Add different file types
- Test larger bundles
- Experiment with different validation options
- Add custom bundle metadata
- Test performance with various file sizes

## Bundle Features Tested

- ✅ File creation and management
- ✅ MIME type detection and validation
- ✅ Entrypoint configuration
- ✅ Bundle metadata handling
- ✅ Compression and size optimization
- ✅ Validation and error reporting
- ✅ Serialization and parsing
- ✅ Round-trip integrity verification
- ✅ Performance measurement
- ✅ Content inspection and preview
