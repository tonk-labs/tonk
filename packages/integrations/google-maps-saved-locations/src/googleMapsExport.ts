#!/usr/bin/env node

import { google } from "googleapis";
import * as dotenv from "dotenv";
import { fileURLToPath } from "url";
import { dirname } from "path";
import * as fs from "fs";
import * as path from "path";
import * as http from "http";
import open from "open";
import { OAuth2Client } from "google-auth-library";

// Get the directory name of the current module
const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

// Token file path
const TOKEN_PATH = path.join(__dirname, "..", "token.json");

// Load environment variables from .env file
dotenv.config({ path: new URL("../.env", import.meta.url).pathname });

/**
 * Get and store new token after prompting for user authorization
 */
async function getNewToken(oAuth2Client: OAuth2Client) {
  const authUrl = oAuth2Client.generateAuthUrl({
    access_type: "offline",
    scope: [
      "https://www.googleapis.com/auth/dataportability.saved.collections",
    ],
    prompt: "consent", // Force to get refresh token
  });

  console.log("Authorize this app by visiting this URL:", authUrl);

  // Create a local server to receive the OAuth callback
  return new Promise((resolve, reject) => {
    const server = http.createServer(async (req, res) => {
      try {
        if (!req.url) {
          throw new Error("No URL in request");
        }

        // Parse the URL and get the code from the query parameters
        const url = new URL(req.url, "http://localhost:4444");
        const code = url.searchParams.get("code");

        if (code) {
          res.writeHead(200, { "Content-Type": "text/html" });
          res.end("Authentication successful! You can close this window.");

          server.close();

          // Get the access token
          const { tokens } = await oAuth2Client.getToken(code);
          oAuth2Client.setCredentials(tokens);

          // Store the token to disk for later program executions
          fs.writeFileSync(TOKEN_PATH, JSON.stringify(tokens));
          console.log("Token stored to", TOKEN_PATH);

          resolve(oAuth2Client);
        } else {
          throw new Error("No code found in the callback URL");
        }
      } catch (error) {
        res.writeHead(400, { "Content-Type": "text/html" });
        res.end("Authentication failed! Please try again.");
        server.close();
        reject(error);
      }
    });

    // Start the server on port 4444
    server.listen(4444, () => {
      console.log(`Local server listening on port 4444`);

      // Open the authorization URL in the default browser
      open(authUrl).catch(() => {
        console.log(
          "Failed to open the browser automatically. Please open the URL manually.",
        );
      });
    });
  });
}

/**
 * Load or create OAuth2 client with the given credentials
 */
async function authorize(credentials: any) {
  const { client_id, client_secret, redirect_uris } =
    credentials.installed || credentials.web;
  const oAuth2Client = new google.auth.OAuth2(
    client_id,
    client_secret,
    redirect_uris[0],
  );

  // Check if we have previously stored a token
  try {
    if (fs.existsSync(TOKEN_PATH)) {
      const token = JSON.parse(fs.readFileSync(TOKEN_PATH, "utf-8"));
      oAuth2Client.setCredentials(token);
      return oAuth2Client;
    } else {
      return await getNewToken(oAuth2Client);
    }
  } catch (error) {
    console.error("Error loading or getting token:", error);
    return await getNewToken(oAuth2Client);
  }
}

async function exportStarredPlaces() {
  // Get credentials from environment variables
  const clientId = process.env.GOOGLE_CLIENT_ID;
  const clientSecret = process.env.GOOGLE_CLIENT_SECRET;
  const redirectUri =
    process.env.GOOGLE_REDIRECT_URI || "http://localhost:4444";

  if (!clientId || !clientSecret) {
    console.error("Error: Missing required environment variables.");
    console.error(
      "Please set GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET in your .env file",
    );
    process.exit(1);
  }

  console.log("Starting Google Maps starred places export...");

  // Initialize the Data Portability API
  const dataportability = google.dataportability("v1beta");

  // Set up OAuth2 client with the required scope
  // const auth = new google.auth.OAuth2(clientId, clientSecret, redirectUri);

  // Set the required scope
  // auth.setCredentials({
  //   scope:
  //     "https://www.googleapis.com/auth/dataportability.maps.starred_places",
  // });

  const credentials = {
    installed: {
      client_id: clientId,
      client_secret: clientSecret,
      redirect_uris: [redirectUri],
    },
  };

  const auth = await authorize(credentials);

  try {
    console.log("Initiating archive job...");
    // Initiate the archive job
    const response = await dataportability.portabilityArchive.initiate({
      auth: auth,
      requestBody: {
        resources: [
          "saved.collections", // Use the correct resource format
        ],
      },
    });

    console.log("Archive job initiated, response:", response.data);

    // Get the archive job ID
    const archiveJobId = response.data.archiveJobId;
    let archiveState;
    do {
      console.log(`Checking status for archive job: ${archiveJobId}`);

      archiveState =
        await dataportability.archiveJobs.getPortabilityArchiveState({
          auth: auth,
          name: `archiveJobs/${archiveJobId}/portabilityArchiveState`,
        });

      console.log(`Current state: ${archiveState.data.state}`);

      if (archiveState.data.state === "COMPLETE") {
        // Download URLs are available in archiveState.data.urls
        console.log("Download URLs:", archiveState.data.urls);
        console.log("all data:", archiveState.data);
        return archiveState.data.urls;
      } else if (archiveState.data.state === "FAILED") {
        console.error("Archive job failed:", archiveState.data.error);
        throw new Error(
          `Archive job failed: ${archiveState.data.error?.message || "Unknown error"}`,
        );
      }

      // Wait for 5 seconds before checking again
      console.log("Waiting 5 seconds before checking again...");
      await new Promise((resolve) => setTimeout(resolve, 5000));
    } while (
      archiveState.data.state === "IN_PROGRESS" ||
      archiveState.data.state === "PENDING"
    );
  } catch (error) {
    console.error("Error:", error);
  }
}

// Run the export function if this file is executed directly
const isMainModule = import.meta.url.endsWith(process.argv[1]);

if (isMainModule) {
  exportStarredPlaces()
    .then((url) => {
      if (url) {
        console.log("Export completed successfully. Download URL:", url);
        process.exit(0);
      }
    })
    .catch((error) => {
      console.error("Export failed:", error);
      process.exit(1);
    });
}
