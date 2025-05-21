"use strict";
var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __setModuleDefault = (this && this.__setModuleDefault) || (Object.create ? (function(o, v) {
    Object.defineProperty(o, "default", { enumerable: true, value: v });
}) : function(o, v) {
    o["default"] = v;
});
var __importStar = (this && this.__importStar) || (function () {
    var ownKeys = function(o) {
        ownKeys = Object.getOwnPropertyNames || function (o) {
            var ar = [];
            for (var k in o) if (Object.prototype.hasOwnProperty.call(o, k)) ar[ar.length] = k;
            return ar;
        };
        return ownKeys(o);
    };
    return function (mod) {
        if (mod && mod.__esModule) return mod;
        var result = {};
        if (mod != null) for (var k = ownKeys(mod), i = 0; i < k.length; i++) if (k[i] !== "default") __createBinding(result, mod, k[i]);
        __setModuleDefault(result, mod);
        return result;
    };
})();
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.startWorker = startWorker;
const keepsync_1 = require("@tonk/keepsync");
const automerge_repo_network_websocket_1 = require("@automerge/automerge-repo-network-websocket");
const automerge_repo_storage_nodefs_1 = require("@automerge/automerge-repo-storage-nodefs");
const http = __importStar(require("http"));
const dotenv_1 = __importDefault(require("dotenv"));
const exportLocations_1 = require("./exportLocations");
const fs_1 = __importDefault(require("fs"));
const path_1 = __importDefault(require("path"));
const csv_parse_1 = require("csv-parse");
const utils_1 = require("./utils");
/**
 * Set up daily location export
 * This function schedules the exportLocations function to run once a day
 * @param keepsyncDocPath Optional path to write locations to in Keepsync
 */
function setupDailyLocationExport(keepsyncDocPath) {
    console.log("Setting up daily location export scheduler");
    // Run immediately on startup
    runLocationExport({ keepsyncDocPath });
    // Schedule to run once a day at 3:00 AM
    const scheduleNextRun = () => {
        const now = new Date();
        const targetTime = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1, // tomorrow
        3, // 3 AM
        0, // 0 minutes
        0);
        // Calculate milliseconds until next run
        const timeUntilNextRun = targetTime.getTime() - now.getTime();
        console.log(`Next location export scheduled for ${targetTime.toISOString()} (in ${Math.floor(timeUntilNextRun / 1000 / 60 / 60)} hours)`);
        // Schedule the next run
        setTimeout(() => {
            runLocationExport({ keepsyncDocPath });
            scheduleNextRun(); // Schedule the next day's run
        }, timeUntilNextRun);
    };
    // Start the scheduling
    scheduleNextRun();
}
/**
 * Convert CSV files to JSON
 * @param directoryPath Path to directory containing CSV files
 * @returns JSON object with location data
 */
async function convertCsvToJson(directoryPath) {
    if (!fs_1.default.existsSync(directoryPath)) {
        console.error(`Directory not found: ${directoryPath}`);
        return {
            locations: [],
            metadata: { count: 0, lastUpdated: new Date().toISOString() },
        };
    }
    // Create a promise to handle the asynchronous CSV parsing
    return new Promise((resolve) => {
        const locations = [];
        const files = fs_1.default.readdirSync(directoryPath);
        let filesProcessed = 0;
        // If no CSV files found, resolve immediately
        if (files.filter((file) => file.endsWith(".csv")).length === 0) {
            return resolve({
                locations: [],
                metadata: {
                    count: 0,
                    lastUpdated: new Date().toISOString(),
                    files: [],
                },
            });
        }
        for (const file of files) {
            if (file.endsWith(".csv")) {
                const filePath = path_1.default.join(directoryPath, file);
                const csvContent = fs_1.default.readFileSync(filePath, "utf-8");
                try {
                    // Parse CSV content
                    (0, csv_parse_1.parse)(csvContent, {
                        columns: true,
                        skip_empty_lines: true,
                    }, (err, output) => {
                        filesProcessed++;
                        if (err) {
                            console.error(`Error parsing CSV file ${file}:`, err);
                        }
                        else {
                            // Add source file information
                            const enhancedRecords = output.map((record) => ({
                                ...record,
                                source_file: file,
                                exported_at: new Date().toISOString(),
                            }));
                            locations.push(...enhancedRecords);
                        }
                        // Check if all files have been processed
                        if (filesProcessed ===
                            files.filter((f) => f.endsWith(".csv")).length) {
                            // Create the final JSON object
                            const result = {
                                locations,
                                metadata: {
                                    count: locations.length,
                                    lastUpdated: new Date().toISOString(),
                                    files: files.filter((f) => f.endsWith(".csv")),
                                },
                            };
                            resolve(result);
                        }
                    });
                }
                catch (error) {
                    console.error(`Error parsing CSV file ${file}:`, error);
                    filesProcessed++;
                    // Check if all files have been processed
                    if (filesProcessed === files.filter((f) => f.endsWith(".csv")).length) {
                        // Create the final JSON object even if some files failed
                        const result = {
                            locations,
                            metadata: {
                                count: locations.length,
                                lastUpdated: new Date().toISOString(),
                                files: files.filter((f) => f.endsWith(".csv")),
                            },
                        };
                        resolve(result);
                    }
                }
            }
        }
    });
}
/**
 * Write locations to Keepsync
 * @param docPath Document path in Keepsync
 * @param locationsData JSON object with location data
 */
async function writeToKeepsync(docPath, locationsData) {
    if (!docPath) {
        console.log("No Keepsync docPath provided, skipping write to Keepsync");
        return;
    }
    try {
        console.log(`Writing ${locationsData.metadata.count} locations to Keepsync at ${docPath}`);
        // Try to read existing document first
        let existingData = await (0, keepsync_1.readDoc)(docPath);
        if (existingData === undefined) {
            console.log("No existing document found at", docPath, "will create a new one");
            existingData = {};
        }
        else {
            console.log("Found existing document at", docPath);
        }
        // Prepare the updated document
        const updatedData = {
            ...(existingData || {}),
            locations: locationsData.locations,
            metadata: locationsData.metadata,
        };
        // Write the updated document
        await (0, keepsync_1.writeDoc)(docPath, updatedData);
        console.log(`Successfully wrote locations to Keepsync at ${docPath}`);
    }
    catch (error) {
        console.error("Error writing to Keepsync:", error);
        throw error;
    }
}
/**
 * Run the location export process
 * @param config Optional configuration with keepsyncDocPath
 */
async function runLocationExport(config) {
    console.log("Running scheduled location export");
    try {
        // Call the exportLocations function
        await (0, exportLocations_1.main)();
        console.log("Scheduled location export completed successfully");
        // If we have a Keepsync docPath, write to Keepsync
        if (config?.keepsyncDocPath) {
            const outputDir = path_1.default.join((0, utils_1.getProjectRoot)(), "exported_locations");
            const locationsData = await convertCsvToJson(outputDir);
            if (locationsData.metadata.count > 0) {
                await writeToKeepsync(config.keepsyncDocPath, locationsData);
            }
            else {
                console.log("No locations found to write to Keepsync");
            }
        }
        else {
            console.log("No Keepsync docPath provided, skipping Keepsync export");
        }
    }
    catch (error) {
        console.error("Error during scheduled location export:", error);
    }
}
// Load environment variables
dotenv_1.default.config();
// Set up global error handlers
process.on("uncaughtException", (err) => {
    console.error(`Uncaught exception: ${err.message}`);
    console.error(err.stack);
});
process.on("unhandledRejection", (reason, promise) => {
    console.error("Unhandled Rejection at:", promise);
    console.error("Reason:", reason);
});
// Log startup information
console.log(`Starting google-maps-locations-worker worker at ${new Date().toISOString()}`);
console.log(`Node.js version: ${process.version}`);
console.log(`Platform: ${process.platform}`);
/**
 * Start the worker with the given configuration
 */
async function startWorker(config) {
    const { port } = config;
    // Configure sync engine
    const SYNC_WS_URL = process.env.SYNC_WS_URL || "ws://localhost:7777/sync";
    const SYNC_URL = process.env.SYNC_URL || "http://localhost:7777";
    const wsAdapter = new automerge_repo_network_websocket_1.BrowserWebSocketClientAdapter(SYNC_WS_URL);
    const engine = (0, keepsync_1.configureSyncEngine)({
        url: SYNC_URL,
        network: [wsAdapter],
        storage: new automerge_repo_storage_nodefs_1.NodeFSStorageAdapter(),
    });
    // Helper function to handle CORS
    const setCorsHeaders = (res) => {
        res.setHeader("Access-Control-Allow-Origin", "*");
        res.setHeader("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
        res.setHeader("Access-Control-Allow-Headers", "Content-Type");
    };
    // Create HTTP server
    const server = http.createServer((req, res) => {
        // Set CORS headers for all responses
        setCorsHeaders(res);
        // Handle preflight OPTIONS requests
        if (req.method === "OPTIONS") {
            res.writeHead(204);
            res.end();
            return;
        }
        // Example endpoint
        if (req.method === "GET" && req.url === "/health") {
            res.writeHead(200, { "Content-Type": "application/json" });
            res.end(JSON.stringify({ status: "ok" }));
            return;
        }
        // Hello endpoint
        if (req.method === "GET" && req.url === "/hello") {
            res.writeHead(200, { "Content-Type": "application/json" });
            res.end(JSON.stringify({ message: "Hello from google-maps-locations-worker!" }));
            return;
        }
        // Manual export locations endpoint
        if (req.method === "POST" && req.url === "/export-locations") {
            console.log("Manual location export triggered");
            // Run the export process asynchronously
            runLocationExport({ keepsyncDocPath: config.keepsyncDocPath })
                .then(() => {
                console.log("Manual location export completed");
            })
                .catch((error) => {
                console.error("Error during manual location export:", error);
            });
            // Immediately return a response to the client
            res.writeHead(200, { "Content-Type": "application/json" });
            res.end(JSON.stringify({
                success: true,
                message: "Location export process started. Check server logs for progress.",
            }));
            return;
        }
        // Main worker endpoint
        if (req.method === "POST" && req.url === "/tonk") {
            let body = "";
            req.on("data", (chunk) => {
                body += chunk.toString();
            });
            req.on("end", async () => {
                try {
                    const data = JSON.parse(body);
                    // Process the request data
                    console.log("Received data:", data);
                    // Send success response
                    res.writeHead(200, { "Content-Type": "application/json" });
                    res.end(JSON.stringify({
                        success: true,
                        message: "Request processed successfully",
                    }));
                }
                catch (error) {
                    console.error("Error processing request:", error);
                    res.writeHead(400, { "Content-Type": "application/json" });
                    res.end(JSON.stringify({ success: false, error: "Invalid data format" }));
                }
            });
        }
        else {
            // Handle other routes
            res.writeHead(404, { "Content-Type": "application/json" });
            res.end(JSON.stringify({ success: false, error: "Not found" }));
        }
    });
    // Start the server
    return new Promise((resolve) => {
        server.listen(port, async () => {
            console.log(`google-maps-locations-worker listening on http://localhost:${port}/tonk`);
            // Initialize the sync engine
            try {
                await engine.whenReady();
                console.log("Keepsync engine is ready");
            }
            catch (error) {
                console.error("Error initializing sync engine:", error);
            }
            // Set up daily location export with keepsyncDocPath
            setupDailyLocationExport(config.keepsyncDocPath);
            // Handle graceful shutdown
            const cleanup = () => {
                console.log("Shutting down...");
                process.exit(0);
            };
            process.on("SIGINT", cleanup);
            process.on("SIGTERM", cleanup);
            resolve(server);
        });
    });
}
// If this file is run directly, start the worker
if (require.main === module) {
    const port = process.env.WORKER_PORT
        ? parseInt(process.env.WORKER_PORT, 10)
        : 5555;
    startWorker({
        port,
        keepsyncDocPath: process.env.KEEPSYNC_DOC_PATH,
    })
        .then(() => console.log(`Worker started on port ${port}`))
        .catch((err) => console.error("Failed to start worker:", err));
}
