import { OAuth2Client } from "google-auth-library";
import http from "http";
import url from "url";
import open from "open";
// server-destroy doesn't have proper ESM support, so we need to use dynamic import
import { createRequire } from "module";
const require = createRequire(import.meta.url);
const destroyer = require("server-destroy");

// Google OAuth scope needed for accessing saved locations
const SCOPES = [
  "https://www.googleapis.com/auth/dataportability.maps.starred_places",
];

export interface GoogleAuthConfig {
  clientId: string;
  clientSecret: string;
  redirectUri?: string;
}

/**
 * Performs the OAuth 2.0 authorization flow with Google
 * @param config OAuth configuration with client ID and secret
 * @returns OAuth2Client with valid credentials
 */
/**
 * Attempts to revoke any existing tokens
 * @param oAuth2Client The OAuth client
 */
async function revokeTokensIfPresent(
  oAuth2Client: OAuth2Client,
): Promise<void> {
  if (oAuth2Client.credentials.access_token) {
    try {
      await oAuth2Client.revokeToken(oAuth2Client.credentials.access_token);
      console.log("Successfully revoked existing tokens");
    } catch (error) {
      console.log("No existing tokens to revoke or revocation failed");
    }
  }
}

export async function authenticateWithGoogle(
  config: GoogleAuthConfig,
): Promise<OAuth2Client> {
  return new Promise((resolve, reject) => {
    // Create OAuth client
    const redirectUri =
      config.redirectUri || "http://localhost:4444/oauth2callback";
    const oAuth2Client = new OAuth2Client(
      config.clientId,
      config.clientSecret,
      redirectUri,
    );

    // Try to revoke any existing tokens first
    try {
      revokeTokensIfPresent(oAuth2Client);
    } catch (error) {
      console.log("Token revocation failed, continuing with authentication");
    }

    // Create a local server to receive the OAuth callback
    const server = http
      .createServer(async (req, res) => {
        try {
          // Handle only the OAuth callback route
          if (!req.url?.includes("/oauth2callback")) {
            res.end("Invalid callback URL");
            return;
          }

          // Close connection after response
          res.end("Authentication successful! You can close this window.");
          server.destroy();

          // Parse the query parameters
          const qs = new url.URL(req.url, "http://localhost:4444").searchParams;
          const code = qs.get("code");

          if (!code) {
            reject(new Error("No code found in the callback"));
            return;
          }

          // Get access and refresh tokens
          const { tokens } = await oAuth2Client.getToken(code);
          oAuth2Client.setCredentials(tokens);
          console.log("Successfully obtained access tokens");

          resolve(oAuth2Client);
        } catch (e) {
          reject(e);
        }
      })
      .listen(4444, () => {
        // Get the authorization URL
        const authorizeUrl = oAuth2Client.generateAuthUrl({
          access_type: "offline",
          scope: SCOPES,
          prompt: "consent",
          include_granted_scopes: false,
        });

        // Open the authorization URL in the default browser
        console.log("Opening browser for Google authentication...");
        open(authorizeUrl, { wait: false }).catch(() => {
          console.log("Failed to open browser automatically.");
          console.log(`Please open this URL in your browser: ${authorizeUrl}`);
        });
      });

    // Enable destroying the server
    destroyer(server);
  });
}

/**
 * Gets user information from Google
 * @param auth Authenticated OAuth2Client
 * @returns Basic information about the authenticated session
 */
export async function getUserInfo(auth: OAuth2Client) {
  // We can't get detailed user info without userinfo scopes
  // Just return the token info which has basic details
  return {
    authenticated: true,
    tokenType: auth.credentials.token_type || "Bearer",
    expiryDate: auth.credentials.expiry_date,
    scopes: SCOPES,
  };
}
