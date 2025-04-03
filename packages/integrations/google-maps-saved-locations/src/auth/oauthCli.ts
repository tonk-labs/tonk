import * as dotenv from "dotenv";
import { authenticateWithGoogle } from "./googleAuth";
import { DataPortabilityClient } from "./dataPortability";
import * as fs from "fs/promises";
import * as path from "path";

dotenv.config();

export interface OAuthConfig {
  clientId: string;
  clientSecret: string;
  outputPath: string;
}

/**
 * Runs the OAuth flow to authenticate with Google and fetch saved locations
 * @param config Configuration for the OAuth flow
 */
export async function runOAuthFlow(config: OAuthConfig): Promise<void> {
  try {
    console.log("Starting Google OAuth authentication flow...");
    
    // Authenticate with Google
    const authClient = await authenticateWithGoogle({
      clientId: config.clientId,
      clientSecret: config.clientSecret,
    });
    
    console.log("Authentication successful!");
    
    // Create Data Portability client
    const dataClient = new DataPortabilityClient({
      auth: authClient,
      tempDir: path.join(process.cwd(), "temp")
    });
    
    // Fetch saved locations
    console.log("Fetching your saved Google Maps locations...");
    const locationsData = await dataClient.fetchSavedLocations();
    
    // Save the results
    await fs.writeFile(
      config.outputPath,
      JSON.stringify(locationsData, null, 2)
    );
    
    console.log(`Saved locations data to ${config.outputPath}`);
  } catch (error) {
    console.error("Error during OAuth flow:", error);
    throw error;
  }
}