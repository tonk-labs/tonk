import { BaseCredentialsManager } from "./utils/baseCredentialsManager";
import { getProjectRoot } from "./utils";

/**
 * Google OAuth Credentials Manager
 * Extends the base credentials manager with Google OAuth specific configurations
 */
export class GoogleOAuthCredentialsManager extends BaseCredentialsManager {
  /**
   * Create a new GoogleOAuthCredentialsManager
   */
  constructor() {
    super([
      {
        name: "Google OAuth Credentials",
        description: "OAuth 2.0 credentials for authenticating with Google APIs",
        filename: "credentials.json",
        instructions: 
          "1. Go to the Google Cloud Console (https://console.cloud.google.com/)\n" +
          "2. Create a new project or select an existing one\n" +
          "3. Enable the required APIs (e.g., Data Portability API)\n" +
          "4. Create OAuth 2.0 credentials (Web application type)\n" +
          "5. Set the redirect URI to http://localhost:4444/oauth2callback\n" +
          "6. Download the credentials JSON file",
        validationFn: (content: string) => {
          try {
            const credentials = JSON.parse(content);
            if (!credentials.web || !credentials.web.client_id || !credentials.web.client_secret) {
              return { 
                valid: false, 
                message: "Invalid credentials format: missing required fields in 'web' object" 
              };
            }
            return { valid: true };
          } catch (e) {
            return { 
              valid: false, 
              message: "Invalid JSON format" 
            };
          }
        },
        sampleContent: 
          '{\n' +
          '  "web": {\n' +
          '    "client_id": "YOUR_CLIENT_ID.apps.googleusercontent.com",\n' +
          '    "project_id": "your-project-id",\n' +
          '    "auth_uri": "https://accounts.google.com/o/oauth2/auth",\n' +
          '    "token_uri": "https://oauth2.googleapis.com/token",\n' +
          '    "auth_provider_x509_cert_url": "https://www.googleapis.com/oauth2/v1/certs",\n' +
          '    "client_secret": "YOUR_CLIENT_SECRET",\n' +
          '    "redirect_uris": ["http://localhost:4444/oauth2callback"]\n' +
          '  }\n' +
          '}'
      }
    ], getProjectRoot());
  }
}

/**
 * Create a Google OAuth credentials manager
 * @returns GoogleOAuthCredentialsManager instance
 */
export function createGoogleOAuthCredentialsManager(): GoogleOAuthCredentialsManager {
  return new GoogleOAuthCredentialsManager();
}
