import fs from "fs";
import path from "path";
import { google } from "googleapis";
import { authenticate } from "@google-cloud/local-auth";
import AdmZip from "adm-zip";
import * as https from "https";

// Configuration
const SCOPES = [
  "https://www.googleapis.com/auth/dataportability.saved.collections",
];

// Path to store token
const TOKEN_PATH = path.join(process.cwd(), "token.json");
// Path to credentials file (you'll need to download this from Google Cloud Console)
const CREDENTIALS_PATH = path.join(process.cwd(), "credentials.json");

/**
 * Load or request authentication
 */
async function authorize() {
  let client: any;

  try {
    // Check if we have previously stored a token
    if (fs.existsSync(TOKEN_PATH)) {
      const content = fs.readFileSync(TOKEN_PATH, "utf-8");

      // Check if the token is valid JSON
      try {
        const credentials = JSON.parse(content);

        // Create OAuth2 client from credentials
        const { client_id, client_secret } = credentials;
        if (client_id && client_secret) {
          const oauth2Client = new google.auth.OAuth2(
            client_id,
            client_secret,
            "http://localhost:4444/oauth2callback",
          );

          oauth2Client.setCredentials(credentials);
          client = oauth2Client;
          console.log("Using existing OAuth2 token");
        } else {
          console.log(
            "Token exists but is missing required fields, re-authenticating...",
          );
          client = await authenticateWithOAuth();
        }
      } catch (e) {
        console.log("Invalid token format, re-authenticating...");
        client = await authenticateWithOAuth();
      }
    } else {
      // If no stored token, authenticate using OAuth
      client = await authenticateWithOAuth();
    }

    return client;
  } catch (err) {
    console.error("Error during authentication:", err);
    throw err;
  }
}

/**
 * Authenticate using OAuth 2.0
 */
async function authenticateWithOAuth() {
  try {
    // Read credentials file
    const credContent = fs.readFileSync(CREDENTIALS_PATH, "utf-8");
    const credentials = JSON.parse(credContent);

    if (!credentials.web) {
      throw new Error("Invalid credentials format: missing 'web' object");
    }

    // Authenticate using local server
    const client = await authenticate({
      scopes: SCOPES,
      keyfilePath: CREDENTIALS_PATH,
    });

    // Save the credentials for future use
    if (client.credentials) {
      // Add client_id and client_secret to the saved token for future use
      const tokenToSave = {
        ...client.credentials,
        client_id: credentials.web.client_id,
        client_secret: credentials.web.client_secret,
      };

      const content = JSON.stringify(tokenToSave);
      fs.writeFileSync(TOKEN_PATH, content);
      console.log(`Token stored to ${TOKEN_PATH}`);
    }

    return client;
  } catch (err) {
    console.error("Error during OAuth authentication:", err);
    throw err;
  }
}

/**
 * Fetch saved locations from Google Maps via Dataportability API
 */
async function fetchSavedLocations(
  auth: any,
): Promise<{ outputDir: string; error?: string }> {
  const dataportability = google.dataportability({
    version: "v1",
    auth,
  });

  let archiveJobId: string;

  try {
    // First, we need to initiate a data export job
    const initResponse = await dataportability.portabilityArchive.initiate({
      requestBody: {
        resources: ["saved.collections"],
      },
    });

    archiveJobId = initResponse.data.archiveJobId;
    console.log(`Export job initiated with ID: ${archiveJobId}`);
  } catch (error: any) {
    // Check if this is the "already exported" error (status code 409)
    if (error.status === 409 && error.errors && error.errors.length > 0) {
      const errorMessage = error.errors[0].message;
      console.log(`Note: ${errorMessage}`);

      // Extract the job ID from the error message using regex
      const jobIdMatch = errorMessage.match(/job ([a-f0-9-]+) matching/);
      if (jobIdMatch && jobIdMatch[1]) {
        archiveJobId = jobIdMatch[1];
        console.log(`Using existing export job ID: ${archiveJobId}`);
      } else {
        console.error(
          "Could not extract job ID from error message. Please try again tomorrow.",
        );
        // Return empty result instead of throwing
        return {
          outputDir: "",
          error: "Rate limited - please try again tomorrow",
        };
      }
    } else {
      // For other errors, log and return
      console.error("Error initiating export:", error.message || error);
      return { outputDir: "", error: "Failed to initiate export" };
    }
  }

  // Poll until the export job is complete
  let state: string;
  try {
    do {
      const statusResponse =
        await dataportability.archiveJobs.getPortabilityArchiveState({
          name: `archiveJobs/${archiveJobId}/portabilityArchiveState`,
        });

      state = statusResponse.data.state || "";
      console.log(`Export status: ${state}`);

      if (state === "IN_PROGRESS") {
        // Wait 5 seconds before checking again
        await new Promise((resolve) => setTimeout(resolve, 5000));
      }
    } while (state === "IN_PROGRESS");

    if (state !== "COMPLETE") {
      console.error(`Export failed with status: ${state}`);
      return { outputDir: "", error: `Export failed with status: ${state}` };
    }
  } catch (error: any) {
    console.error("Error checking export status:", error.message || error);
    return { outputDir: "", error: "Failed to check export status" };
  }

  // Get the download URLs from the completed job
  try {
    const archiveState =
      await dataportability.archiveJobs.getPortabilityArchiveState({
        name: `archiveJobs/${archiveJobId}/portabilityArchiveState`,
      });

    // Download the data from the provided URLs
    const urls = archiveState.data.urls || [];
    if (urls.length === 0) {
      console.error("No download URLs available");
      return { outputDir: "", error: "No download URLs available" };
    }

    console.log(`Found ${urls.length} download URLs`);

    // Create a directory to store all CSV files
    const outputDir = path.join(process.cwd(), "exported_locations");
    if (!fs.existsSync(outputDir)) {
      fs.mkdirSync(outputDir, { recursive: true });
    }

    // Process each URL (usually there's just one, but we'll handle multiple just in case)
    for (let i = 0; i < urls.length; i++) {
      const downloadUrl = urls[i];
      console.log(
        `Downloading from URL ${i + 1}/${urls.length}: ${downloadUrl}`,
      );

      // Create a temporary file to store the downloaded archive
      const tempFilePath = path.join(process.cwd(), `temp_archive_${i}.zip`);

      try {
        // Download the file
        await new Promise<void>((resolve, reject) => {
          const file = fs.createWriteStream(tempFilePath);
          https
            .get(downloadUrl, (response: any) => {
              if (response.statusCode !== 200) {
                reject(new Error(`Failed to download: ${response.statusCode}`));
                return;
              }

              response.pipe(file);
              file.on("finish", () => {
                file.close();
                console.log(`Download completed to ${tempFilePath}`);
                resolve();
              });
            })
            .on("error", (err: Error) => {
              fs.unlink(tempFilePath, () => {});
              reject(err);
            });
        });

        console.log(`Extracting archive: ${tempFilePath}`);

        // Unzip the archive
        const zip = new AdmZip(tempFilePath);
        const extractDir = path.join(process.cwd(), `extracted_${i}`);

        // Extract all files
        zip.extractAllTo(extractDir, true);
        console.log(`Extracted to ${extractDir}`);

        // Find all CSV files recursively
        const findCsvFiles = (
          dir: string,
          fileList: string[] = [],
        ): string[] => {
          const files = fs.readdirSync(dir);

          files.forEach((file) => {
            const filePath = path.join(dir, file);
            if (fs.statSync(filePath).isDirectory()) {
              findCsvFiles(filePath, fileList);
            } else if (file.toLowerCase().endsWith(".csv")) {
              fileList.push(filePath);
            }
          });

          return fileList;
        };

        const csvFiles = findCsvFiles(extractDir);
        console.log(`Found ${csvFiles.length} CSV files`);

        // Copy all CSV files to the output directory
        csvFiles.forEach((csvFile, index) => {
          const fileName = path.basename(csvFile);
          const destPath = path.join(outputDir, `${index + 1}_${fileName}`);
          fs.copyFileSync(csvFile, destPath);
          console.log(`Copied ${fileName} to ${destPath}`);
        });

        // Clean up temporary files
        fs.rmSync(tempFilePath, { force: true });
        fs.rmSync(extractDir, { recursive: true, force: true });
      } catch (error) {
        console.error(`Error processing archive ${i}:`, error);
        // Continue with next URL even if this one fails
      }
    }

    console.log(
      `All downloads processed. CSV files available in: ${outputDir}`,
    );

    // Return the path to the output directory instead of the data
    return { outputDir };
  } catch (error: any) {
    console.error(
      "Error downloading or processing files:",
      error.message || error,
    );
    return { outputDir: "", error: "Failed to download or process files" };
  }
}

/**
 * Main function
 */
export async function main() {
  try {
    // Check if credentials file exists
    if (!fs.existsSync(CREDENTIALS_PATH)) {
      console.error(`Credentials file not found at ${CREDENTIALS_PATH}`);
      console.error(
        "Please create a credentials.json file with your Google OAuth 2.0 client credentials",
      );
      return;
    }

    // Validate credentials format
    try {
      const credContent = fs.readFileSync(CREDENTIALS_PATH, "utf-8");
      const credentials = JSON.parse(credContent);

      if (
        !credentials.web ||
        !credentials.web.client_id ||
        !credentials.web.client_secret
      ) {
        console.error(
          "Invalid credentials format: missing required fields in 'web' object",
        );
        console.error(
          "Please ensure your credentials.json contains a valid OAuth 2.0 client configuration",
        );
        return;
      }
    } catch (e) {
      console.error("Error reading or parsing credentials file:", e);
      return;
    }

    console.log("Authenticating...");
    const auth = await authorize();

    console.log("Fetching saved locations...");
    const result = await fetchSavedLocations(auth);

    if (result.error) {
      console.log(`Note: ${result.error}`);
      return;
    }

    if (result.outputDir) {
      console.log(`All CSV files have been exported to: ${result.outputDir}`);
      console.log("Process completed successfully!");
    }
  } catch (error) {
    console.error("An error occurred:", error);
  }
}
