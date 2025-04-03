import { OAuth2Client } from "google-auth-library";
import { google } from "googleapis";
import * as fs from "fs/promises";
import * as path from "path";
import { OutputFormat, Location } from "../convert/schema";

// Define the Google Data Portability API interface
interface DataPortabilityOptions {
  auth: OAuth2Client;
  tempDir?: string;
}

/**
 * Client for interacting with Google's Data Portability API
 * to retrieve saved locations from Google Maps
 */
export class DataPortabilityClient {
  private auth: OAuth2Client;
  private tempDir: string;

  constructor(options: DataPortabilityOptions) {
    this.auth = options.auth;
    this.tempDir = options.tempDir || "./temp";
  }

  /**
   * Initiates a data export request for Google Maps saved locations
   * @returns The archive ID of the export
   */
  async requestMapsDataExport(): Promise<string> {
    console.log("Requesting Google Maps data export...");

    // Initialize the Data Transfer API
    const dataTransfer = google.datatransfer({
      version: "v1",
      auth: this.auth,
    });

    // Request export of Google Maps saved locations
    const exportRequest = {
      exportDataRequest: {
        dataCategories: ["MAPS_YOUR_PLACES"],
        destinationFormat: "JSON",
      },
    };

    try {
      const response = await dataTransfer.archives.create({
        requestBody: exportRequest,
      });

      if (!response.data.id) {
        throw new Error("Failed to create data export request");
      }

      return response.data.id;
    } catch (error) {
      console.error("Error requesting data export:", error);
      throw error;
    }
  }

  /**
   * Polls the status of a data export request until it's complete
   * @param archiveId The ID of the export archive
   * @returns The download URL when the export is ready
   */
  async waitForExportCompletion(archiveId: string): Promise<string> {
    console.log("Waiting for export to complete...");

    const dataTransfer = google.datatransfer({
      version: "v1",
      auth: this.auth,
    });

    // Poll until the export is complete
    let isComplete = false;
    let downloadUrl = "";

    while (!isComplete) {
      const response = await dataTransfer.archives.get({
        archiveId: archiveId,
      });

      const status = response.data.status;

      if (status === "COMPLETE") {
        isComplete = true;
        downloadUrl = response.data.downloadUrl || "";
        if (!downloadUrl) {
          throw new Error("Export completed but no download URL provided");
        }
      } else if (status === "FAILED") {
        throw new Error(
          "Export failed: " + (response.data.errorMessage || "Unknown error"),
        );
      } else {
        // Wait before polling again
        console.log(`Export status: ${status}. Waiting...`);
        await new Promise((resolve) => setTimeout(resolve, 5000));
      }
    }

    return downloadUrl;
  }

  /**
   * Downloads and processes the exported data
   * @param downloadUrl URL to download the exported data
   * @returns Processed location data
   */
  async downloadAndProcessExport(downloadUrl: string): Promise<OutputFormat> {
    console.log("Downloading and processing exported data...");

    // Ensure temp directory exists
    await fs.mkdir(this.tempDir, { recursive: true });

    // Download the archive
    const response = await fetch(downloadUrl, {
      headers: {
        Authorization: `Bearer ${this.auth.credentials.access_token}`,
      },
    });

    if (!response.ok) {
      throw new Error(`Failed to download export: ${response.statusText}`);
    }

    // Save the archive
    const archivePath = path.join(this.tempDir, "maps_export.zip");
    const buffer = await response.arrayBuffer();
    await fs.writeFile(archivePath, Buffer.from(buffer));

    // Extract and process the saved locations
    // This is a simplified version - in reality, you'd need to extract the ZIP
    // and parse the JSON files inside to find the saved locations
    const locations = await this.extractLocationsFromArchive(archivePath);

    return {
      locations: locations,
    };
  }

  /**
   * Extracts location data from the downloaded archive
   * @param archivePath Path to the downloaded archive
   * @returns Processed location data
   */
  private async extractLocationsFromArchive(
    archivePath: string,
  ): Promise<Record<string, Location>> {
    // In a real implementation, you would:
    // 1. Extract the ZIP file
    // 2. Find and parse the JSON files containing saved locations
    // 3. Transform the data into the expected format

    // This is a placeholder implementation
    console.log(`Extracting locations from ${archivePath}`);

    // Mock implementation - in reality, you'd parse the actual files
    const locations: Record<string, Location> = {};

    // Clean up temp files
    // await fs.unlink(archivePath);

    return locations;
  }

  /**
   * Complete workflow to fetch Google Maps saved locations
   * @returns Processed location data
   */
  async fetchSavedLocations(): Promise<OutputFormat> {
    try {
      // Request the data export
      const archiveId = await this.requestMapsDataExport();

      // Wait for the export to complete
      const downloadUrl = await this.waitForExportCompletion(archiveId);

      // Download and process the export
      return await this.downloadAndProcessExport(downloadUrl);
    } catch (error) {
      console.error("Error fetching saved locations:", error);
      throw error;
    }
  }
}
