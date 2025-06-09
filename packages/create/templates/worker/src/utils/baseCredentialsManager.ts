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
import fs from "fs";
import { promises as fsPromises } from "fs";
import path from "path";
import readline from "readline";

/**
 * Interface for credential configuration
 */
export interface CredentialConfig {
  name: string;
  description: string;
  filename: string;
  instructions: string;
  validationFn?: (content: string) => { valid: boolean; message?: string };
  sampleContent?: string;
  header?: string; // Header name to include this credential in (e.g., "Authorization", "X-API-Key")
  param?: string; // URL parameter name to include this credential as (e.g., "api_key", "token")
}

/**
 * Base class to manage credentials for workers
 * This manages credential files that are bundled with the worker.
 * Credentials are stored in a 'credentials' directory relative to the worker bundle.
 */
export class BaseCredentialsManager {
  protected configs: CredentialConfig[];
  protected credentialsDir: string;

  /**
   * Create a new BaseCredentialsManager
   * @param configs Array of credential configurations
   * @param credentialsDir Optional custom credentials directory (defaults to './credentials' relative to worker)
   */
  constructor(configs: CredentialConfig[], credentialsDir?: string) {
    this.configs = configs;
    // Default to a credentials directory relative to the worker's location
    this.credentialsDir = credentialsDir || this.getDefaultCredentialsDir();
  }

  /**
   * Get the default credentials directory for the worker
   * Uses the root of the beefy-worker project as the base
   */
  private getDefaultCredentialsDir(): string {
    // Navigate up from the utils directory to the project root
    // This file is in src/utils/, so we need to go up to find the project root
    const currentDir = __dirname || process.cwd();

    // If we're in dist/utils, go up 2 levels; if we're in src/utils, go up 2 levels
    const projectRoot = path.resolve(currentDir, "../../");
    return path.join(projectRoot, "creds");
  }

  /**
   * Check if all required credential files exist
   * @returns Promise resolving to object with status and missing credentials
   */
  public async checkCredentials(): Promise<{
    complete: boolean;
    missing: string[];
    missingConfigs: CredentialConfig[];
  }> {
    const missing: string[] = [];
    const missingConfigs: CredentialConfig[] = [];

    for (const config of this.configs) {
      const filePath = path.join(this.credentialsDir, config.filename);
      try {
        await fsPromises.access(filePath);
      } catch {
        missing.push(config.name);
        missingConfigs.push(config);
      }
    }

    return {
      complete: missing.length === 0,
      missing,
      missingConfigs,
    };
  }

  /**
   * Set up credentials interactively
   * @returns Promise that resolves when setup is complete
   */
  public async setupCredentials(): Promise<void> {
    const { complete, missingConfigs } = await this.checkCredentials();

    if (complete) {
      console.log("✅ All required credentials are already set up.");
      return;
    }

    console.log("Setting up credentials...");

    const rl = readline.createInterface({
      input: process.stdin,
      output: process.stdout,
    });

    const question = (query: string): Promise<string> => {
      return new Promise((resolve) => {
        rl.question(query, (answer) => {
          resolve(answer);
        });
      });
    };

    for (const config of missingConfigs) {
      console.log(`\n📄 Setting up ${config.name}:`);
      console.log(config.description);
      console.log("\nInstructions:");
      console.log(config.instructions);

      if (config.sampleContent) {
        console.log("\nSample content:");
        console.log(config.sampleContent);
      }

      let validContent = false;
      let content = "";

      while (!validContent) {
        const option = await question(
          "\nHow would you like to provide this credential?\n" +
            "1. Enter the content directly\n" +
            "2. Specify a path to an existing file\n" +
            "3. Skip (you'll need to add this file manually later)\n" +
            "Choose an option (1-3): "
        );

        if (option === "1") {
          console.log(
            `\nEnter the content for ${config.filename} (type 'END' on a new line when finished):`
          );
          const lines: string[] = [];
          let line = "";

          while (line !== "END") {
            line = await question("");
            if (line !== "END") {
              lines.push(line);
            }
          }

          content = lines.join("\n");
        } else if (option === "2") {
          const filePath = await question(
            "\nEnter the path to the existing file: "
          );
          try {
            content = await fsPromises.readFile(filePath, "utf-8");
          } catch (error) {
            console.error(`Error reading file: ${error.message}`);
            continue;
          }
        } else if (option === "3") {
          console.log(
            `\nSkipping ${config.name}. You'll need to add this file manually later.`
          );
          break;
        } else {
          console.log("Invalid option. Please choose 1, 2, or 3.");
          continue;
        }

        // Validate content if a validation function is provided
        if (config.validationFn) {
          const validation = config.validationFn(content);
          if (!validation.valid) {
            console.error(`Invalid content: ${validation.message}`);
            continue;
          }
        }

        // Write the content to the file
        const filePath = path.join(this.credentialsDir, config.filename);
        try {
          // Ensure the credentials directory exists
          try {
            await fsPromises.access(this.credentialsDir);
          } catch {
            await fsPromises.mkdir(this.credentialsDir, { recursive: true });
          }
          await fsPromises.writeFile(filePath, content);
          console.log(`✅ Successfully created ${config.filename}`);
          validContent = true;
        } catch (error) {
          console.error(`Error writing file: ${error.message}`);
        }
      }
    }

    rl.close();
    console.log("\n🎉 Credential setup complete!");
  }

  /**
   * Get the path to a credential file
   * @param filename Name of the credential file
   * @returns Absolute path to the credential file
   */
  public getCredentialPath(filename: string): string {
    return path.join(this.credentialsDir, filename);
  }

  /**
   * Load credential content from file
   * @param filename Name of the credential file
   * @returns Promise resolving to the credential content or null if file doesn't exist
   */
  public async loadCredential(filename: string): Promise<string | null> {
    try {
      const filePath = this.getCredentialPath(filename);
      try {
        await fsPromises.access(filePath);
      } catch {
        return null;
      }
      const content = await fsPromises.readFile(filePath, "utf-8");
      return content.trim();
    } catch (error) {
      console.error(`Error loading credential file ${filename}:`, error);
      return null;
    }
  }

  /**
   * Load all available credentials
   * @returns Promise resolving to object with credential names as keys and content as values
   */
  public async loadAllCredentials(): Promise<Record<string, string | null>> {
    const credentials: Record<string, string | null> = {};

    for (const config of this.configs) {
      credentials[config.name] = await this.loadCredential(config.filename);
    }

    return credentials;
  }

  /**
   * Get credential by name
   * @param name The credential name (not filename)
   * @returns Promise resolving to the credential content or null if not found
   */
  public async getCredentialByName(name: string): Promise<string | null> {
    const config = this.configs.find((c) => c.name === name);
    if (!config) {
      return null;
    }
    return await this.loadCredential(config.filename);
  }

  /**
   * Get info about all configured credentials
   * @returns Promise resolving to array of credential info objects
   */
  public async getCredentialInfo(): Promise<
    Array<{
      name: string;
      description: string;
      filename: string;
      exists: boolean;
      filePath: string;
    }>
  > {
    const results = [];

    for (const config of this.configs) {
      const filePath = this.getCredentialPath(config.filename);
      let exists = false;
      try {
        await fsPromises.access(filePath);
        exists = true;
      } catch {
        exists = false;
      }

      results.push({
        name: config.name,
        description: config.description,
        filename: config.filename,
        exists,
        filePath,
      });
    }

    return results;
  }
}
