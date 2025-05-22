import { configureSyncEngine, readDoc, writeDoc } from "@tonk/keepsync";
import { NetworkAdapterInterface } from "@automerge/automerge-repo";
import { BrowserWebSocketClientAdapter } from "@automerge/automerge-repo-network-websocket";
import { NodeFSStorageAdapter } from "@automerge/automerge-repo-storage-nodefs";
import * as http from "http";
import dotenv from "dotenv";
import { main as exportLocations } from "./exportLocations";
import fs from "fs";
import path from "path";
import { parse } from "csv-parse";
import { getProjectRoot } from "./utils";

/**
 * Set up daily location export
 * This function schedules the exportLocations function to run once a day
 * @param keepsyncDocPath Optional path to write locations to in Keepsync
 */
function setupDailyLocationExport(keepsyncDocPath?: string) {
  console.log("Setting up daily location export scheduler");

  // Run immediately on startup
  runLocationExport({ keepsyncDocPath });

  // Schedule to run once a day at 3:00 AM
  const scheduleNextRun = () => {
    const now = new Date();
    const targetTime = new Date(
      now.getFullYear(),
      now.getMonth(),
      now.getDate() + 1, // tomorrow
      3, // 3 AM
      0, // 0 minutes
      0, // 0 seconds
    );

    // Calculate milliseconds until next run
    const timeUntilNextRun = targetTime.getTime() - now.getTime();

    console.log(
      `Next location export scheduled for ${targetTime.toISOString()} (in ${Math.floor(timeUntilNextRun / 1000 / 60 / 60)} hours)`,
    );

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
async function convertCsvToJson(
  directoryPath: string,
): Promise<Record<string, any>> {
  if (!fs.existsSync(directoryPath)) {
    console.error(`Directory not found: ${directoryPath}`);
    return {
      locations: [],
      metadata: { count: 0, lastUpdated: new Date().toISOString() },
    };
  }

  // Create a promise to handle the asynchronous CSV parsing
  return new Promise((resolve) => {
    const locations: any[] = [];
    const files = fs.readdirSync(directoryPath);
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
        const filePath = path.join(directoryPath, file);
        const csvContent = fs.readFileSync(filePath, "utf-8");

        try {
          // Parse CSV content
          parse(
            csvContent,
            {
              columns: true,
              skip_empty_lines: true,
            },
            (err, output) => {
              filesProcessed++;

              if (err) {
                console.error(`Error parsing CSV file ${file}:`, err);
              } else {
                // Add source file information
                const enhancedRecords = output.map((record: any) => ({
                  ...record,
                  source_file: file,
                  exported_at: new Date().toISOString(),
                }));

                locations.push(...enhancedRecords);
              }

              // Check if all files have been processed
              if (
                filesProcessed ===
                files.filter((f) => f.endsWith(".csv")).length
              ) {
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
            },
          );
        } catch (error) {
          console.error(`Error parsing CSV file ${file}:`, error);
          filesProcessed++;

          // Check if all files have been processed
          if (
            filesProcessed === files.filter((f) => f.endsWith(".csv")).length
          ) {
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
async function writeToKeepsync(
  docPath: string,
  locationsData: Record<string, any>,
): Promise<void> {
  if (!docPath) {
    console.log("No Keepsync docPath provided, skipping write to Keepsync");
    return;
  }

  try {
    console.log(
      `Writing ${locationsData.metadata.count} locations to Keepsync at ${docPath}`,
    );

    // Try to read existing document first
    let existingData = await readDoc(docPath);

    if (existingData === undefined) {
      console.log(
        "No existing document found at",
        docPath,
        "will create a new one",
      );
      existingData = {};
    } else {
      console.log("Found existing document at", docPath);
    }

    // Prepare the updated document
    const updatedData = {
      ...((existingData as object) || {}),
      locations: locationsData.locations,
      metadata: locationsData.metadata,
    };

    // Write the updated document
    await writeDoc(docPath, updatedData);

    console.log(`Successfully wrote locations to Keepsync at ${docPath}`);
  } catch (error) {
    console.error("Error writing to Keepsync:", error);
    throw error;
  }
}

/**
 * Run the location export process
 * @param config Optional configuration with keepsyncDocPath
 */
async function runLocationExport(config?: { keepsyncDocPath?: string }) {
  console.log("Running scheduled location export");
  try {
    // Call the exportLocations function
    await exportLocations();
    console.log("Scheduled location export completed successfully");

    // If we have a Keepsync docPath, write to Keepsync
    if (config?.keepsyncDocPath) {
      const outputDir = path.join(getProjectRoot(), "exported_locations");

      const locationsData = await convertCsvToJson(outputDir);
      if (locationsData.metadata.count > 0) {
        await writeToKeepsync(config.keepsyncDocPath, locationsData);
      } else {
        console.log("No locations found to write to Keepsync");
      }
    } else {
      console.log("No Keepsync docPath provided, skipping Keepsync export");
    }
  } catch (error) {
    console.error("Error during scheduled location export:", error);
  }
}

// Load environment variables
dotenv.config();

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
console.log(
  `Starting google-maps-locations-worker worker at ${new Date().toISOString()}`,
);
console.log(`Node.js version: ${process.version}`);
console.log(`Platform: ${process.platform}`);

/**
 * Configuration for the worker
 */
interface WorkerConfig {
  port: number;
  keepsyncDocPath?: string; // Optional path to write locations to in Keepsync
}

/**
 * Start the worker with the given configuration
 */
export async function startWorker(config: WorkerConfig): Promise<http.Server> {
  const { port } = config;

  // Configure sync engine
  const SYNC_WS_URL = process.env.SYNC_WS_URL || "ws://localhost:7777/sync";
  const SYNC_URL = process.env.SYNC_URL || "http://localhost:7777";

  const wsAdapter = new BrowserWebSocketClientAdapter(SYNC_WS_URL);
  const engine = configureSyncEngine({
    url: SYNC_URL,
    network: [wsAdapter as any as NetworkAdapterInterface],
    storage: new NodeFSStorageAdapter(),
  });

  // Helper function to handle CORS
  const setCorsHeaders = (res: http.ServerResponse) => {
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

    // Health endpoint
    if (req.method === "GET" && req.url === "/health") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ status: "ok" }));
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
      res.end(
        JSON.stringify({
          success: true,
          message:
            "Location export process started. Check server logs for progress.",
        }),
      );
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
          res.end(
            JSON.stringify({
              success: true,
              message: "Request processed successfully",
            }),
          );
        } catch (error) {
          console.error("Error processing request:", error);
          res.writeHead(400, { "Content-Type": "application/json" });
          res.end(
            JSON.stringify({ success: false, error: "Invalid data format" }),
          );
        }
      });
    } else {
      // Handle other routes
      res.writeHead(404, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ success: false, error: "Not found" }));
    }
  });

  // Start the server
  return new Promise((resolve) => {
    server.listen(port, async () => {
      console.log(
        `google-maps-locations-worker listening on http://localhost:${port}/tonk`,
      );

      // Initialize the sync engine
      try {
        await engine.whenReady();
        console.log("Keepsync engine is ready");
      } catch (error) {
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
