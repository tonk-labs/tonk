/**
 * ⚠️  CORE TEMPLATE FILE - DO NOT DELETE! ⚠️
 *
 * This file is part of the Tonk worker template infrastructure.
 * It provides file system watching capabilities for workers that need to
 * monitor file changes and automatically process them into keepsync documents.
 *
 * USAGE: Import and use this when you need to:
 * - Watch local files for changes
 * - Automatically process file content when files are added/modified
 * - Transform file data and store it in keepsync
 *
 * Even if your current worker doesn't use file watching, other workers might!
 * Keep this file for the template integrity.
 */
import { IOHandler, IOResult } from "./types";
import { BaseCredentialsManager } from "../utils/baseCredentialsManager";
import { keepsyncLogger } from "../utils/keepsyncLogger";

/**
 * Credential information for web request authentication
 */
export interface WebRequestCredentialInfo {
  available: boolean;
  credentials: Record<string, string | null>;
  credentialConfigs: Array<{
    name: string;
    description: string;
    filename: string;
    exists: boolean;
    filePath: string;
    header?: string;
    param?: string;
  }>;
}

/**
 * Web request handler implementation
 * Handles HTTP requests to read data from web endpoints with optional credential management
 */
export class WebRequestHandler
  implements
    IOHandler<BaseCredentialsManager, WebRequestCredentialInfo, string>
{
  private credentialsManager?: BaseCredentialsManager;

  /**
   * Initialize the web request handler with an optional credentials manager
   * @param credentialsManager The BaseCredentialsManager instance (optional)
   */
  async init(credentialsManager?: BaseCredentialsManager): Promise<void> {
    this.credentialsManager = credentialsManager;
  }

  /**
   * Get information about available credentials
   * @returns Credential information object
   */
  async info(): Promise<WebRequestCredentialInfo> {
    if (!this.credentialsManager) {
      return {
        available: false,
        credentials: {},
        credentialConfigs: [],
      };
    }

    const credentials = await this.credentialsManager.loadAllCredentials();
    const baseCredentialConfigs =
      await this.credentialsManager.getCredentialInfo();
    const configs = this.credentialsManager["configs"]; // Access configs to get header/param info

    // Merge the base info with header/param configuration
    const credentialConfigs = baseCredentialConfigs.map((baseConfig) => {
      const config = configs.find((c) => c.filename === baseConfig.filename);
      return {
        ...baseConfig,
        header: config?.header,
        param: config?.param,
      };
    });

    return {
      available: Object.values(credentials).some((cred) => cred !== null),
      credentials,
      credentialConfigs,
    };
  }

  /**
   * Read data from a web endpoint
   * @param url The web endpoint URL to fetch data from
   */
  async read(url: string): Promise<IOResult<string>> {
    try {
      const headers: Record<string, string> = {
        "Content-Type": "application/json",
      };

      // Build the final URL with any credential parameters (if credentials are available)
      let finalUrl = url;

      // Add authentication headers and URL parameters only if credentials are available
      if (this.credentialsManager) {
        finalUrl = await this.addAuthCredentials(headers, url);
      }

      const response = await fetch(finalUrl, {
        method: "GET",
        headers,
      });

      if (!response.ok) {
        const error = `HTTP ${response.status}: ${response.statusText}`;
        // Log failed read operation
        await keepsyncLogger.logRead("web", url, {
          error,
          status: response.status,
        });
        return {
          success: false,
          error: `HTTP ${response.status}: ${response.statusText}`,
        };
      }

      const data = await response.text();

      // Log successful read operation
      await keepsyncLogger.logRead("web", url, data);

      return {
        success: true,
        data,
      };
    } catch (error) {
      const errorMessage =
        error instanceof Error
          ? error.message
          : "Failed to fetch from web endpoint";
      // Log failed read operation
      await keepsyncLogger.logRead("web", url, { error: errorMessage });
      return {
        success: false,
        error:
          error instanceof Error
            ? error.message
            : "Failed to fetch from web endpoint",
      };
    }
  }

  /**
   * Add authentication credentials based on configuration
   * @param headers Headers object to modify
   * @param url The base URL
   * @returns The final URL with any credential parameters added
   */
  private async addAuthCredentials(
    headers: Record<string, string>,
    url: string
  ): Promise<string> {
    if (!this.credentialsManager) return url;

    const credentials = await this.credentialsManager.loadAllCredentials();
    const configs = this.credentialsManager["configs"]; // Access configs from the manager

    const urlParams = new URLSearchParams();
    let hasParams = false;

    for (const config of configs) {
      const credentialValue = credentials[config.name];

      // Skip if credential is not available
      if (!credentialValue) continue;

      // Add to headers if header field is specified
      if (config.header) {
        headers[config.header] = credentialValue;
      }

      // Add to URL parameters if param field is specified
      if (config.param) {
        urlParams.append(config.param, credentialValue);
        hasParams = true;
      }
    }

    // Append URL parameters if any were added
    if (hasParams) {
      const separator = url.includes("?") ? "&" : "?";
      return `${url}${separator}${urlParams.toString()}`;
    }

    return url;
  }
}

/**
 * Factory function to create a new web request handler
 */
export function createWebRequestHandler(): WebRequestHandler {
  return new WebRequestHandler();
}
