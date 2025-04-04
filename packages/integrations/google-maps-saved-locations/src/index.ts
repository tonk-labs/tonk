// Export OAuth and Data Portability functionality
export { authenticateWithGoogle, getUserInfo, GoogleAuthConfig } from './auth/googleAuth';
export { DataPortabilityClient, DataPortabilityOptions } from './auth/dataPortability';

// Export CSV conversion functionality
export { convertLocations } from './convert/convertLocations';
export { ConversionConfig, createConfig } from './convert/config';
export { OutputFormat, Location } from './convert/schema';

// Export CLI functionality
export { runOAuthFlow, OAuthConfig } from './auth/oauthCli';

/**
 * Main function to fetch Google Maps saved locations using OAuth
 * @param clientId Google OAuth client ID
 * @param clientSecret Google OAuth client secret
 * @param outputPath Path to save the output JSON file
 * @returns The location data
 */
export async function fetchGoogleMapsSavedLocations({
  clientId,
  clientSecret,
  outputPath = 'locations-output.json'
}: {
  clientId: string;
  clientSecret: string;
  outputPath?: string;
}) {
  // Authenticate with Google
  const authClient = await authenticateWithGoogle({
    clientId,
    clientSecret,
  });
  
  // Create Data Portability client
  const dataClient = new DataPortabilityClient({
    auth: authClient,
  });
  
  // Fetch saved locations
  return await dataClient.fetchSavedLocations();
}