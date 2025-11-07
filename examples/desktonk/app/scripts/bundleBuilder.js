"use strict";
var __awaiter = (this && this.__awaiter) || function (thisArg, _arguments, P, generator) {
    function adopt(value) { return value instanceof P ? value : new P(function (resolve) { resolve(value); }); }
    return new (P || (P = Promise))(function (resolve, reject) {
        function fulfilled(value) { try { step(generator.next(value)); } catch (e) { reject(e); } }
        function rejected(value) { try { step(generator["throw"](value)); } catch (e) { reject(e); } }
        function step(result) { result.done ? resolve(result.value) : adopt(result.value).then(fulfilled, rejected); }
        step((generator = generator.apply(thisArg, _arguments || [])).next());
    });
};
var __generator = (this && this.__generator) || function (thisArg, body) {
    var _ = { label: 0, sent: function() { if (t[0] & 1) throw t[1]; return t[1]; }, trys: [], ops: [] }, f, y, t, g = Object.create((typeof Iterator === "function" ? Iterator : Object).prototype);
    return g.next = verb(0), g["throw"] = verb(1), g["return"] = verb(2), typeof Symbol === "function" && (g[Symbol.iterator] = function() { return this; }), g;
    function verb(n) { return function (v) { return step([n, v]); }; }
    function step(op) {
        if (f) throw new TypeError("Generator is already executing.");
        while (g && (g = 0, op[0] && (_ = 0)), _) try {
            if (f = 1, y && (t = op[0] & 2 ? y["return"] : op[0] ? y["throw"] || ((t = y["return"]) && t.call(y), 0) : y.next) && !(t = t.call(y, op[1])).done) return t;
            if (y = 0, t) op = [op[0] & 2, t.value];
            switch (op[0]) {
                case 0: case 1: t = op; break;
                case 4: _.label++; return { value: op[1], done: false };
                case 5: _.label++; y = op[1]; op = [0]; continue;
                case 7: op = _.ops.pop(); _.trys.pop(); continue;
                default:
                    if (!(t = _.trys, t = t.length > 0 && t[t.length - 1]) && (op[0] === 6 || op[0] === 2)) { _ = 0; continue; }
                    if (op[0] === 3 && (!t || (op[1] > t[0] && op[1] < t[3]))) { _.label = op[1]; break; }
                    if (op[0] === 6 && _.label < t[1]) { _.label = t[1]; t = op; break; }
                    if (t && _.label < t[2]) { _.label = t[2]; _.ops.push(op); break; }
                    if (t[2]) _.ops.pop();
                    _.trys.pop(); continue;
            }
            op = body.call(thisArg, _);
        } catch (e) { op = [6, e]; y = 0; } finally { f = t = 0; }
        if (op[0] & 5) throw op[1]; return { value: op[0] ? op[1] : void 0, done: true };
    }
};
Object.defineProperty(exports, "__esModule", { value: true });
var core_1 = require("@tonk/core");
var fs_1 = require("fs");
var path_1 = require("path");
var mime_1 = require("mime");
/**
 * Enhanced MIME type detection function (extracted from service-worker.ts)
 */
function determineMimeType(path) {
    if (!path) {
        return 'application/octet-stream';
    }
    // Clean the path - remove query parameters and fragments
    var cleanPath = path.split('?')[0].split('#')[0];
    // Extract file extension
    var lastDot = cleanPath.lastIndexOf('.');
    var extension = lastDot !== -1 ? cleanPath.substring(lastDot).toLowerCase() : '';
    // Common web file types with their MIME types
    var mimeTypes = {
        // Web documents
        '.html': 'text/html',
        '.htm': 'text/html',
        '.xhtml': 'application/xhtml+xml',
        // Stylesheets
        '.css': 'text/css',
        '.scss': 'text/css',
        '.sass': 'text/css',
        '.less': 'text/css',
        // JavaScript
        '.js': 'text/javascript',
        '.mjs': 'text/javascript',
        '.jsx': 'text/javascript',
        '.ts': 'text/typescript',
        '.tsx': 'text/typescript',
        '.cjs': 'text/javascript',
        '.esm': 'text/javascript',
        // JSON
        '.json': 'application/json',
        '.jsonld': 'application/ld+json',
        // Images
        '.png': 'image/png',
        '.jpg': 'image/jpeg',
        '.jpeg': 'image/jpeg',
        '.gif': 'image/gif',
        '.svg': 'image/svg+xml',
        '.webp': 'image/webp',
        '.ico': 'image/x-icon',
        '.bmp': 'image/bmp',
        '.tiff': 'image/tiff',
        '.tif': 'image/tiff',
        '.avif': 'image/avif',
        '.heic': 'image/heic',
        '.heif': 'image/heif',
        // Fonts
        '.woff': 'font/woff',
        '.woff2': 'font/woff2',
        '.ttf': 'font/ttf',
        '.otf': 'font/otf',
        '.eot': 'application/vnd.ms-fontobject',
        // Audio
        '.mp3': 'audio/mpeg',
        '.wav': 'audio/wav',
        '.ogg': 'audio/ogg',
        '.m4a': 'audio/mp4',
        '.aac': 'audio/aac',
        // Video
        '.mp4': 'video/mp4',
        '.webm': 'video/webm',
        '.ogv': 'video/ogg',
        '.avi': 'video/x-msvideo',
        '.mov': 'video/quicktime',
        // Text formats
        '.txt': 'text/plain',
        '.md': 'text/markdown',
        '.xml': 'application/xml',
        '.csv': 'text/csv',
        // Archives
        '.zip': 'application/zip',
        '.tar': 'application/x-tar',
        '.gz': 'application/gzip',
        '.7z': 'application/x-7z-compressed',
        // Documents
        '.pdf': 'application/pdf',
        '.doc': 'application/msword',
        '.docx': 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
        '.xls': 'application/vnd.ms-excel',
        '.xlsx': 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
        // Web assembly
        '.wasm': 'application/wasm',
        // Manifest files
        '.manifest': 'text/cache-manifest',
        '.webmanifest': 'application/manifest+json',
    };
    // Check our custom mapping first
    if (mimeTypes[extension]) {
        return mimeTypes[extension];
    }
    // Fall back to the mime library
    var mimeType = mime_1.default.getType(cleanPath);
    if (mimeType) {
        return mimeType;
    }
    // Special handling for files without extensions
    if (!extension) {
        // Check if it looks like a common web file based on the path
        if (cleanPath.endsWith('/') || cleanPath === '' || cleanPath === 'index') {
            return 'text/html'; // Assume directory requests want HTML
        }
        // Check for common extensionless files
        var fileName = cleanPath.split('/').pop() || '';
        var commonFiles = {
            robots: 'text/plain',
            sitemap: 'application/xml',
            favicon: 'image/x-icon',
            manifest: 'application/manifest+json',
        };
        if (commonFiles[fileName]) {
            return commonFiles[fileName];
        }
    }
    // Ultimate fallback
    return 'application/octet-stream';
}
/**
 * Converts a base64 string back to Uint8Array (kept for backward compatibility)
 */
function base64ToUint8Array(base64) {
    return new Uint8Array(Buffer.from(base64, 'base64'));
}
/**
 * Recursively reads all files from a directory and returns them as an array
 * of objects containing the relative path and file data as Uint8Array
 */
function readDistFiles(distPath) {
    var files = [];
    function walkDirectory(currentPath, basePath) {
        var items = (0, fs_1.readdirSync)(currentPath);
        for (var _i = 0, items_1 = items; _i < items_1.length; _i++) {
            var item = items_1[_i];
            var fullPath = (0, path_1.join)(currentPath, item);
            var stat = (0, fs_1.statSync)(fullPath);
            if (stat.isDirectory()) {
                // Recursively walk subdirectories
                walkDirectory(fullPath, basePath);
            }
            else if (stat.isFile()) {
                // Read file and store with relative path
                var relativePath = '/' + (0, path_1.relative)(basePath, fullPath).replace(/\\/g, '/');
                var fileData = new Uint8Array((0, fs_1.readFileSync)(fullPath));
                files.push({ relativePath: relativePath, fileData: fileData });
            }
        }
    }
    walkDirectory(distPath, distPath);
    return files;
}
/**
 * Recursively gets all files from the VFS using a queue-based approach
 */
function getAllFilesFromVfs(tonk_1) {
    return __awaiter(this, arguments, void 0, function (tonk, startPath) {
        var allFiles, queue, currentPath, entries, _i, entries_1, entry, fullPath, error_1;
        if (startPath === void 0) { startPath = '/'; }
        return __generator(this, function (_a) {
            switch (_a.label) {
                case 0:
                    allFiles = [];
                    queue = [startPath];
                    _a.label = 1;
                case 1:
                    if (!(queue.length > 0)) return [3 /*break*/, 6];
                    currentPath = queue.shift();
                    _a.label = 2;
                case 2:
                    _a.trys.push([2, 4, , 5]);
                    return [4 /*yield*/, tonk.listDirectory(currentPath)];
                case 3:
                    entries = _a.sent();
                    for (_i = 0, entries_1 = entries; _i < entries_1.length; _i++) {
                        entry = entries_1[_i];
                        fullPath = currentPath === '/'
                            ? "/".concat(entry.name)
                            : "".concat(currentPath, "/").concat(entry.name);
                        if (entry.type === 'directory') {
                            // Add directory to queue for processing
                            queue.push(fullPath);
                        }
                        else if (entry.type === 'document') {
                            // Add file to the list
                            allFiles.push(fullPath);
                        }
                    }
                    return [3 /*break*/, 5];
                case 4:
                    error_1 = _a.sent();
                    // If we can't list a directory, skip it silently
                    console.warn("Could not list directory ".concat(currentPath, ":"), error_1);
                    return [3 /*break*/, 5];
                case 5: return [3 /*break*/, 1];
                case 6: return [2 /*return*/, allFiles];
            }
        });
    });
}
/**
 * Creates a bundle from the dist/ folder
 */
function createBundle(outputPath_1) {
    return __awaiter(this, arguments, void 0, function (outputPath, copyToServer) {
        var tonk, projectName, distPath, distFiles, _i, distFiles_1, _a, relativePath, fileData, cleanRelativePath, modifiedPath, mimeType, bytes, bundlePath, serverDir, serverBundlePath, error_2;
        if (copyToServer === void 0) { copyToServer = false; }
        return __generator(this, function (_b) {
            switch (_b.label) {
                case 0:
                    _b.trys.push([0, 7, , 8]);
                    console.log('Initializing TonkCore...');
                    return [4 /*yield*/, core_1.TonkCore.create()];
                case 1:
                    tonk = _b.sent();
                    projectName = process.cwd().split('/').pop() || 'tonk-project';
                    console.log('Reading files from dist/ folder...');
                    distPath = (0, path_1.join)(process.cwd(), 'dist');
                    if (!(0, fs_1.existsSync)(distPath)) {
                        throw new Error('dist/ folder not found. Please run "npm run build" first.');
                    }
                    distFiles = readDistFiles(distPath);
                    console.log("Found ".concat(distFiles.length, " files to bundle"));
                    _i = 0, distFiles_1 = distFiles;
                    _b.label = 2;
                case 2:
                    if (!(_i < distFiles_1.length)) return [3 /*break*/, 5];
                    _a = distFiles_1[_i], relativePath = _a.relativePath, fileData = _a.fileData;
                    cleanRelativePath = relativePath.startsWith('/')
                        ? relativePath.substring(1)
                        : relativePath;
                    modifiedPath = "/app/".concat(projectName, "/").concat(cleanRelativePath);
                    mimeType = determineMimeType(relativePath);
                    console.log("Adding file: ".concat(modifiedPath, " (").concat(mimeType, ")"));
                    // Use createFileWithBytes with MIME type
                    return [4 /*yield*/, tonk.createFileWithBytes(modifiedPath, { mime: mimeType }, fileData)];
                case 3:
                    // Use createFileWithBytes with MIME type
                    _b.sent();
                    _b.label = 4;
                case 4:
                    _i++;
                    return [3 /*break*/, 2];
                case 5:
                    console.log('Creating bundle...');
                    return [4 /*yield*/, tonk.toBytes()];
                case 6:
                    bytes = _b.sent();
                    bundlePath = outputPath || (0, path_1.join)(process.cwd(), "".concat(projectName, ".tonk"));
                    (0, fs_1.writeFileSync)(bundlePath, bytes);
                    console.log("Bundle created successfully: ".concat(bundlePath));
                    console.log("Bundle size: ".concat(bytes.length, " bytes"));
                    // Copy to server directory if requested
                    if (copyToServer) {
                        serverDir = (0, path_1.join)(process.cwd(), 'server');
                        if ((0, fs_1.existsSync)(serverDir)) {
                            serverBundlePath = (0, path_1.join)(serverDir, "".concat(projectName, ".tonk"));
                            (0, fs_1.writeFileSync)(serverBundlePath, bytes);
                            console.log("Bundle copied to server directory: ".concat(serverBundlePath));
                        }
                        else {
                            console.warn('Server directory not found, skipping copy to server/');
                        }
                    }
                    return [3 /*break*/, 8];
                case 7:
                    error_2 = _b.sent();
                    console.error('Error creating bundle:', error_2);
                    process.exit(1);
                    return [3 /*break*/, 8];
                case 8: return [2 /*return*/];
            }
        });
    });
}
/**
 * Unpacks a bundle into a specified folder
 */
function unpackBundle(bundlePath, outputDir) {
    return __awaiter(this, void 0, void 0, function () {
        var bundleData, tonk, files, _i, files_1, filePath, fileResult, fileData, cleanPath, outputPath, fileDir, error_3;
        return __generator(this, function (_a) {
            switch (_a.label) {
                case 0:
                    _a.trys.push([0, 7, , 8]);
                    console.log("Unpacking bundle: ".concat(bundlePath));
                    if (!(0, fs_1.existsSync)(bundlePath)) {
                        throw new Error("Bundle file not found: ".concat(bundlePath));
                    }
                    bundleData = (0, fs_1.readFileSync)(bundlePath);
                    return [4 /*yield*/, core_1.TonkCore.fromBytes(bundleData)];
                case 1:
                    tonk = _a.sent();
                    return [4 /*yield*/, getAllFilesFromVfs(tonk, '/')];
                case 2:
                    files = _a.sent();
                    console.log("Found ".concat(files.length, " files in bundle"));
                    // Create output directory if it doesn't exist
                    if (!(0, fs_1.existsSync)(outputDir)) {
                        (0, fs_1.mkdirSync)(outputDir, { recursive: true });
                    }
                    _i = 0, files_1 = files;
                    _a.label = 3;
                case 3:
                    if (!(_i < files_1.length)) return [3 /*break*/, 6];
                    filePath = files_1[_i];
                    console.log("Extracting: ".concat(filePath));
                    return [4 /*yield*/, tonk.readFile(filePath)];
                case 4:
                    fileResult = _a.sent();
                    fileData = void 0;
                    if (fileResult.bytes) {
                        // File stored with createFileWithBytes - bytes property contains base64 string
                        fileData = base64ToUint8Array(fileResult.bytes);
                    }
                    else {
                        // Legacy format: content is base64 string
                        fileData = base64ToUint8Array(fileResult.content);
                    }
                    cleanPath = filePath.startsWith('/')
                        ? filePath.substring(1)
                        : filePath;
                    outputPath = (0, path_1.join)(outputDir, cleanPath);
                    fileDir = (0, path_1.dirname)(outputPath);
                    if (!(0, fs_1.existsSync)(fileDir)) {
                        (0, fs_1.mkdirSync)(fileDir, { recursive: true });
                    }
                    // Write the file
                    (0, fs_1.writeFileSync)(outputPath, fileData);
                    _a.label = 5;
                case 5:
                    _i++;
                    return [3 /*break*/, 3];
                case 6:
                    console.log("Bundle unpacked successfully to: ".concat(outputDir));
                    return [3 /*break*/, 8];
                case 7:
                    error_3 = _a.sent();
                    console.error('Error unpacking bundle:', error_3);
                    process.exit(1);
                    return [3 /*break*/, 8];
                case 8: return [2 /*return*/];
            }
        });
    });
}
/**
 * Lists directories and files in a bundle
 */
function listBundle(bundlePath) {
    return __awaiter(this, void 0, void 0, function () {
        var bundleData, tonk, files, directories, _i, files_2, filePath, dir, sortedDirs, _a, sortedDirs_1, dir, dirFiles, _b, dirFiles_1, file, fileName, error_4;
        return __generator(this, function (_c) {
            switch (_c.label) {
                case 0:
                    _c.trys.push([0, 3, , 4]);
                    console.log("Listing contents of bundle: ".concat(bundlePath));
                    if (!(0, fs_1.existsSync)(bundlePath)) {
                        throw new Error("Bundle file not found: ".concat(bundlePath));
                    }
                    bundleData = (0, fs_1.readFileSync)(bundlePath);
                    return [4 /*yield*/, core_1.TonkCore.fromBytes(bundleData)];
                case 1:
                    tonk = _c.sent();
                    return [4 /*yield*/, getAllFilesFromVfs(tonk, '/')];
                case 2:
                    files = _c.sent();
                    console.log("\nBundle contains ".concat(files.length, " files:\n"));
                    directories = new Map();
                    for (_i = 0, files_2 = files; _i < files_2.length; _i++) {
                        filePath = files_2[_i];
                        dir = (0, path_1.dirname)(filePath);
                        if (!directories.has(dir)) {
                            directories.set(dir, []);
                        }
                        directories.get(dir).push(filePath);
                    }
                    sortedDirs = Array.from(directories.keys()).sort();
                    for (_a = 0, sortedDirs_1 = sortedDirs; _a < sortedDirs_1.length; _a++) {
                        dir = sortedDirs_1[_a];
                        console.log("\uD83D\uDCC1 ".concat(dir, "/"));
                        dirFiles = directories.get(dir).sort();
                        for (_b = 0, dirFiles_1 = dirFiles; _b < dirFiles_1.length; _b++) {
                            file = dirFiles_1[_b];
                            fileName = file.substring(file.lastIndexOf('/') + 1);
                            console.log("  \uD83D\uDCC4 ".concat(fileName));
                        }
                        console.log();
                    }
                    return [3 /*break*/, 4];
                case 3:
                    error_4 = _c.sent();
                    console.error('Error listing bundle:', error_4);
                    process.exit(1);
                    return [3 /*break*/, 4];
                case 4: return [2 /*return*/];
            }
        });
    });
}
/**
 * Displays help information
 */
function showHelp() {
    console.log("\nBundle Builder - Create, unpack, and inspect Tonk bundles\n\nUsage:\n  npm run bundle-builder [command] [options]\n\nCommands:\n  create [output]     Create a bundle from dist/ folder\n                      Optional: specify output path (default: latergram.tonk)\n  \n  unpack <bundle> <output-dir>\n                      Unpack a bundle into the specified directory\n  \n  list <bundle>       List all directories and files in a bundle\n\nExamples:\n  npm run bundle-builder create\n  npm run bundle-builder create my-app.tonk\n  npm run bundle-builder create --copy-to-server\n  npm run bundle-builder create my-app.tonk --copy-to-server\n  npm run bundle-builder unpack test-wasm.tonk ./unpacked\n  npm run bundle-builder list test-wasm.tonk\n\nOptions:\n  --copy-to-server   Copy the created bundle to the server/ directory\n  --help, -h         Show this help message\n");
}
/**
 * Main function to handle command-line arguments
 */
function main() {
    return __awaiter(this, void 0, void 0, function () {
        var args, command, _a, createArgs, copyToServer, outputPath;
        return __generator(this, function (_b) {
            switch (_b.label) {
                case 0:
                    args = process.argv.slice(2);
                    if (args.length === 0 || args.includes('--help') || args.includes('-h')) {
                        showHelp();
                        return [2 /*return*/];
                    }
                    command = args[0];
                    _a = command;
                    switch (_a) {
                        case 'create': return [3 /*break*/, 1];
                        case 'unpack': return [3 /*break*/, 3];
                        case 'list': return [3 /*break*/, 5];
                    }
                    return [3 /*break*/, 7];
                case 1:
                    createArgs = args.slice(1);
                    copyToServer = createArgs.includes('--copy-to-server');
                    outputPath = createArgs.find(function (arg) { return !arg.startsWith('--'); });
                    return [4 /*yield*/, createBundle(outputPath, copyToServer)];
                case 2:
                    _b.sent();
                    return [3 /*break*/, 8];
                case 3:
                    if (args.length < 3) {
                        console.error('Error: unpack command requires bundle path and output directory');
                        console.log('Usage: npm run bundle-builder unpack <bundle> <output-dir>');
                        process.exit(1);
                    }
                    return [4 /*yield*/, unpackBundle(args[1], args[2])];
                case 4:
                    _b.sent();
                    return [3 /*break*/, 8];
                case 5:
                    if (args.length < 2) {
                        console.error('Error: list command requires bundle path');
                        console.log('Usage: npm run bundle-builder list <bundle>');
                        process.exit(1);
                    }
                    return [4 /*yield*/, listBundle(args[1])];
                case 6:
                    _b.sent();
                    return [3 /*break*/, 8];
                case 7:
                    console.error("Error: Unknown command '".concat(command, "'"));
                    showHelp();
                    process.exit(1);
                    _b.label = 8;
                case 8: return [2 /*return*/];
            }
        });
    });
}
// Run the main function
main();
