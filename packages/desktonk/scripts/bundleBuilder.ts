import {
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, relative } from "node:path";
import { TonkCore } from "@tonk/core";
import mime from "mime";

/**
 * Enhanced MIME type detection function (extracted from service-worker.ts)
 */
function determineMimeType(path?: string): string {
  if (!path) {
    return "application/octet-stream";
  }

  // Clean the path - remove query parameters and fragments
  const cleanPath = path.split("?")[0].split("#")[0];

  // Extract file extension
  const lastDot = cleanPath.lastIndexOf(".");
  const extension =
    lastDot !== -1 ? cleanPath.substring(lastDot).toLowerCase() : "";

  // Common web file types with their MIME types
  const mimeTypes: Record<string, string> = {
    // Web documents
    ".html": "text/html",
    ".htm": "text/html",
    ".xhtml": "application/xhtml+xml",

    // Stylesheets
    ".css": "text/css",
    ".scss": "text/css",
    ".sass": "text/css",
    ".less": "text/css",

    // JavaScript
    ".js": "text/javascript",
    ".mjs": "text/javascript",
    ".jsx": "text/javascript",
    ".ts": "text/typescript",
    ".tsx": "text/typescript",
    ".cjs": "text/javascript",
    ".esm": "text/javascript",

    // JSON
    ".json": "application/json",
    ".jsonld": "application/ld+json",

    // Images
    ".png": "image/png",
    ".jpg": "image/jpeg",
    ".jpeg": "image/jpeg",
    ".gif": "image/gif",
    ".svg": "image/svg+xml",
    ".webp": "image/webp",
    ".ico": "image/x-icon",
    ".bmp": "image/bmp",
    ".tiff": "image/tiff",
    ".tif": "image/tiff",
    ".avif": "image/avif",
    ".heic": "image/heic",
    ".heif": "image/heif",

    // Fonts
    ".woff": "font/woff",
    ".woff2": "font/woff2",
    ".ttf": "font/ttf",
    ".otf": "font/otf",
    ".eot": "application/vnd.ms-fontobject",

    // Audio
    ".mp3": "audio/mpeg",
    ".wav": "audio/wav",
    ".ogg": "audio/ogg",
    ".m4a": "audio/mp4",
    ".aac": "audio/aac",

    // Video
    ".mp4": "video/mp4",
    ".webm": "video/webm",
    ".ogv": "video/ogg",
    ".avi": "video/x-msvideo",
    ".mov": "video/quicktime",

    // Text formats
    ".txt": "text/plain",
    ".md": "text/markdown",
    ".xml": "application/xml",
    ".csv": "text/csv",

    // Archives
    ".zip": "application/zip",
    ".tar": "application/x-tar",
    ".gz": "application/gzip",
    ".7z": "application/x-7z-compressed",

    // Documents
    ".pdf": "application/pdf",
    ".doc": "application/msword",
    ".docx":
      "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ".xls": "application/vnd.ms-excel",
    ".xlsx":
      "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",

    // Web assembly
    ".wasm": "application/wasm",

    // Manifest files
    ".manifest": "text/cache-manifest",
    ".webmanifest": "application/manifest+json",
  };

  // Check our custom mapping first
  if (mimeTypes[extension]) {
    return mimeTypes[extension];
  }

  // Fall back to the mime library
  const mimeType = mime.getType(cleanPath);
  if (mimeType) {
    return mimeType;
  }

  // Special handling for files without extensions
  if (!extension) {
    // Check if it looks like a common web file based on the path
    if (cleanPath.endsWith("/") || cleanPath === "" || cleanPath === "index") {
      return "text/html"; // Assume directory requests want HTML
    }

    // Check for common extensionless files
    const fileName = cleanPath.split("/").pop() || "";
    const commonFiles: Record<string, string> = {
      robots: "text/plain",
      sitemap: "application/xml",
      favicon: "image/x-icon",
      manifest: "application/manifest+json",
    };

    if (commonFiles[fileName]) {
      return commonFiles[fileName];
    }
  }

  // Ultimate fallback
  return "application/octet-stream";
}

/**
 * Converts a base64 string back to Uint8Array
 */
function base64ToUint8Array(base64: string): Uint8Array {
  return new Uint8Array(Buffer.from(base64, "base64"));
}

/**
 * Recursively reads all files from a directory and returns them as an array
 * of objects containing the relative path and file data as Uint8Array
 */
function readDistFiles(
  distPath: string,
): Array<{ relativePath: string; fileData: Uint8Array }> {
  const files: Array<{ relativePath: string; fileData: Uint8Array }> = [];

  function walkDirectory(currentPath: string, basePath: string) {
    const items = readdirSync(currentPath);

    for (const item of items) {
      const fullPath = join(currentPath, item);
      const stat = statSync(fullPath);

      if (stat.isDirectory()) {
        // Recursively walk subdirectories
        walkDirectory(fullPath, basePath);
      } else if (stat.isFile()) {
        // Read file and store with relative path
        const relativePath = `/${relative(basePath, fullPath).replace(/\\/g, "/")}`;
        const fileData = new Uint8Array(readFileSync(fullPath));
        files.push({ relativePath, fileData });
      }
    }
  }

  walkDirectory(distPath, distPath);
  return files;
}

/**
 * Recursively gets all files from the VFS using a queue-based approach
 */
async function getAllFilesFromVfs(
  // biome-ignore lint/suspicious/noExplicitAny: Tonk instance
  tonk: any,
  startPath: string = "/",
): Promise<string[]> {
  const allFiles: string[] = [];
  const queue: string[] = [startPath];

  while (queue.length > 0) {
    const currentPath = queue.shift();
    if (!currentPath) continue;

    try {
      const entries = await tonk.listDirectory(currentPath);

      for (const entry of entries) {
        const fullPath =
          currentPath === "/"
            ? `/${entry.name}`
            : `${currentPath}/${entry.name}`;

        if (entry.type === "directory") {
          // Add directory to queue for processing
          queue.push(fullPath);
        } else if (entry.type === "document") {
          // Add file to the list
          allFiles.push(fullPath);
        }
      }
    } catch (error) {
      // If we can't list a directory, skip it silently
      console.warn(`Could not list directory ${currentPath}:`, error);
    }
  }

  return allFiles;
}

/**
 * Creates a bundle from the dist/ folder
 */
async function createBundle(
  outputPath?: string,
  copyToServer: boolean = false,
  bundleName?: string,
  networkUri?: string,
) {
  try {
    console.log("Initializing TonkCore...");

    // Initialize TonkCore
    const tonk = await TonkCore.create();

    // Use provided bundle name, or default to 'app'
    const projectName = bundleName || "app";

    // The entrypoint must be the directory containing the app,
    // e.g. if files are in /my-app/index.html, entrypoint is "my-app"
    const entrypoint = projectName;

    console.log("Reading files from dist/ folder...");

    // Read all files from dist folder
    const distPath = join(process.cwd(), "dist");

    if (!existsSync(distPath)) {
      throw new Error(
        'dist/ folder not found. Please run "npm run build" first.',
      );
    }

    const distFiles = readDistFiles(distPath);

    console.log(`Found ${distFiles.length} files to bundle`);

    // Read sample files from sample_files directory and add to /desktonk
    const sampleFilesPath = join(process.cwd(), "..", "sample_files");
    let sampleFiles: Array<{ relativePath: string; fileData: Uint8Array }> = [];

    if (existsSync(sampleFilesPath)) {
      console.log("Reading sample files from sample_files/ folder...");
      const rawSampleFiles = readDistFiles(sampleFilesPath);
      // Prefix sample files with /desktonk/
      sampleFiles = rawSampleFiles.map(({ relativePath, fileData }) => ({
        relativePath: `/desktonk${relativePath}`,
        fileData,
      }));
      console.log(
        `Found ${sampleFiles.length} sample files to add to /desktonk`,
      );
    } else {
      console.warn("sample_files/ folder not found, skipping sample files");
    }

    // Combine dist files and sample files
    const allFiles = [...distFiles, ...sampleFiles];

    // Store each file in the TonkCore VFS at root level
    for (const { relativePath, fileData } of allFiles) {
      // Remove leading slash from relativePath to avoid double slashes
      const cleanRelativePath = relativePath.startsWith("/")
        ? relativePath.substring(1)
        : relativePath;

      // Sample files already have /desktonk prefix, don't add projectName
      const modifiedPath = relativePath.startsWith("/desktonk")
        ? relativePath
        : `/${projectName}/${cleanRelativePath}`;

      const mimeType = determineMimeType(relativePath);
      console.log(`Adding file: ${modifiedPath} (${mimeType})`);
      // Use createFileWithBytes with MIME type
      await tonk.createFileWithBytes(
        modifiedPath,
        { mime: mimeType },
        fileData,
      );
    }

    console.log("Creating bundle...");

    // Get the bundle bytes
    // Note: Config object must match Rust struct expectations.
    // If network_uris is required by Rust but optional in TS types, we must provide it.
    // The error "missing field network_uris" suggests snake_case is required by the WASM bridge.
    // The error "Bundle has no entrypoints" suggests we need to ensure entrypoints is populated correctly.
    const bytes = await tonk.toBytes({
      entrypoints: [entrypoint],
      networkUris: networkUri ? [networkUri] : [],
      // @ts-expect-error - Rust WASM expects snake_case
      network_uris: networkUri ? [networkUri] : [],
    });

    // Save the bundle as a .tonk file
    const bundlePath = outputPath || join(process.cwd(), `${projectName}.tonk`);

    writeFileSync(bundlePath, bytes);

    console.log(`Bundle created successfully: ${bundlePath}`);
    console.log(`Bundle size: ${bytes.length} bytes`);

    // Copy to server directory if requested
    if (copyToServer) {
      const serverDir = join(process.cwd(), "server");
      if (existsSync(serverDir)) {
        const serverBundlePath = join(serverDir, `${projectName}.tonk`);
        writeFileSync(serverBundlePath, bytes);
        console.log(`Bundle copied to server directory: ${serverBundlePath}`);
      } else {
        console.warn("Server directory not found, skipping copy to server/");
      }
    }
  } catch (error) {
    console.error("Error creating bundle:", error);
    process.exit(1);
  }
}

/**
 * Unpacks a bundle into a specified folder
 */
async function unpackBundle(bundlePath: string, outputDir: string) {
  try {
    console.log(`Unpacking bundle: ${bundlePath}`);

    if (!existsSync(bundlePath)) {
      throw new Error(`Bundle file not found: ${bundlePath}`);
    }

    // Read the bundle file
    const bundleData = readFileSync(bundlePath);

    // Initialize TonkCore from the bundle
    const tonk = await TonkCore.fromBytes(bundleData);

    // Get all files from the VFS
    const files = await getAllFilesFromVfs(tonk, "/");

    console.log(`Found ${files.length} files in bundle`);

    // Create output directory if it doesn't exist
    if (!existsSync(outputDir)) {
      mkdirSync(outputDir, { recursive: true });
    }

    // Extract each file
    for (const filePath of files) {
      console.log(`Extracting: ${filePath}`);

      const fileResult = await tonk.readFile(filePath);

      // Handle DocumentData format: { content: JsonObject, bytes: string }
      if (!fileResult.bytes) {
        throw new Error(
          `File ${filePath} has no bytes data - corrupted bundle?`,
        );
      }
      const fileData = base64ToUint8Array(fileResult.bytes);

      // Use the actual VFS path structure (remove leading slash for file system)
      const cleanPath = filePath.startsWith("/")
        ? filePath.substring(1)
        : filePath;
      const outputPath = join(outputDir, cleanPath);

      // Create directory structure if needed
      const fileDir = dirname(outputPath);
      if (!existsSync(fileDir)) {
        mkdirSync(fileDir, { recursive: true });
      }

      // Write the file
      writeFileSync(outputPath, fileData);
    }

    console.log(`Bundle unpacked successfully to: ${outputDir}`);
  } catch (error) {
    console.error("Error unpacking bundle:", error);
    process.exit(1);
  }
}

/**
 * Lists directories and files in a bundle
 */
async function listBundle(bundlePath: string) {
  try {
    console.log(`Listing contents of bundle: ${bundlePath}`);

    if (!existsSync(bundlePath)) {
      throw new Error(`Bundle file not found: ${bundlePath}`);
    }

    // Read the bundle file
    const bundleData = readFileSync(bundlePath);

    // Initialize TonkCore from the bundle
    const tonk = await TonkCore.fromBytes(bundleData);

    // Get all files from the VFS
    const files = await getAllFilesFromVfs(tonk, "/");

    console.log(`\nBundle contains ${files.length} files:\n`);

    // Group files by directory for better visualization
    const directories = new Map<string, string[]>();

    for (const filePath of files) {
      const dir = dirname(filePath);
      if (!directories.has(dir)) {
        directories.set(dir, []);
      }
      directories.get(dir)?.push(filePath);
    }

    // Sort directories and display
    const sortedDirs = Array.from(directories.keys()).sort();

    for (const dir of sortedDirs) {
      console.log(`📁 ${dir}/`);
      const dirFiles = directories.get(dir)?.sort() || [];
      for (const file of dirFiles) {
        const fileName = file.substring(file.lastIndexOf("/") + 1);
        console.log(`  📄 ${fileName}`);
      }
      console.log();
    }
  } catch (error) {
    console.error("Error listing bundle:", error);
    process.exit(1);
  }
}

/**
 * Displays help information
 */
function showHelp() {
  console.log(`
Bundle Builder - Create, unpack, and inspect Tonk bundles

Usage:
  npm run bundle-builder [command] [options]

Commands:
  create [output]     Create a bundle from dist/ folder
                      Optional: specify output path (default: app.tonk)

  unpack <bundle> <output-dir>
                      Unpack a bundle into the specified directory

  list <bundle>       List all directories and files in a bundle

Examples:
  npm run bundle-builder create
  npm run bundle-builder create my-app.tonk
  npm run bundle-builder create --name myapp
  npm run bundle-builder create --copy-to-server
  npm run bundle-builder create --network-uri http://localhost:8081
  npm run bundle-builder create my-app.tonk --name myapp --copy-to-server
  npm run bundle-builder unpack test-wasm.tonk ./unpacked
  npm run bundle-builder list test-wasm.tonk

Options:
  --name, -n <name>        Set the bundle name/entrypoint (default: app)
  --network-uri, -u <uri>  Set a network URI for the bundle manifest
  --copy-to-server         Copy the created bundle to the server/ directory
  --help, -h               Show this help message
`);
}

/**
 * Main function to handle command-line arguments
 */
async function main() {
  const args = process.argv.slice(2);

  if (args.length === 0 || args.includes("--help") || args.includes("-h")) {
    showHelp();
    return;
  }

  const command = args[0];

  switch (command) {
    case "create": {
      // Parse arguments for create command
      const createArgs = args.slice(1);
      const copyToServer = createArgs.includes("--copy-to-server");

      // Parse --name or -n argument
      let bundleName: string | undefined;
      const nameIndex = createArgs.findIndex(
        (arg) => arg === "--name" || arg === "-n",
      );
      if (nameIndex !== -1 && createArgs[nameIndex + 1]) {
        bundleName = createArgs[nameIndex + 1];
      }

      // Parse --network-uri or -u argument
      let networkUri: string | undefined;
      const uriIndex = createArgs.findIndex(
        (arg) => arg === "--network-uri" || arg === "-u",
      );
      if (uriIndex !== -1 && createArgs[uriIndex + 1]) {
        networkUri = createArgs[uriIndex + 1];
      }

      // Output path is any non-flag argument that isn't the bundle name or uri value
      const outputPath = createArgs.find(
        (arg, idx) =>
          !arg.startsWith("--") &&
          !arg.startsWith("-") &&
          idx !== nameIndex + 1 &&
          idx !== uriIndex + 1,
      );

      await createBundle(outputPath, copyToServer, bundleName, networkUri);
      break;
    }

    case "unpack": {
      if (args.length < 3) {
        console.error(
          "Error: unpack command requires bundle path and output directory",
        );
        console.log(
          "Usage: npm run bundle-builder unpack <bundle> <output-dir>",
        );
        process.exit(1);
      }
      await unpackBundle(args[1], args[2]);
      break;
    }

    case "list": {
      if (args.length < 2) {
        console.error("Error: list command requires bundle path");
        console.log("Usage: npm run bundle-builder list <bundle>");
        process.exit(1);
      }
      await listBundle(args[1]);
      break;
    }

    default:
      console.error(`Error: Unknown command '${command}'`);
      showHelp();
      process.exit(1);
  }
}

// Run the main function
main();
