import fs from "fs";
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
  header?: string; // For API keys that go in headers
  urlParam?: string; // For API keys that go in URL parameters
}

/**
 * Base class to manage credentials for workers
 * This can be used by any worker that needs to manage credential files
 */
export class BaseCredentialsManager {
  protected configs: CredentialConfig[];
  protected projectRoot: string;

  /**
   * Create a new BaseCredentialsManager
   * @param configs Array of credential configurations
   * @param projectRoot Root directory of the project
   */
  constructor(configs: CredentialConfig[], projectRoot: string) {
    this.configs = configs;
    this.projectRoot = projectRoot;
  }

  /**
   * Check if all required credential files exist
   * @returns Object with status and missing credentials
   */
  public checkCredentials(): { 
    complete: boolean; 
    missing: string[];
    missingConfigs: CredentialConfig[];
  } {
    const missing: string[] = [];
    const missingConfigs: CredentialConfig[] = [];

    for (const config of this.configs) {
      const filePath = path.join(this.projectRoot, config.filename);
      if (!fs.existsSync(filePath)) {
        missing.push(config.name);
        missingConfigs.push(config);
      }
    }

    return {
      complete: missing.length === 0,
      missing,
      missingConfigs
    };
  }

  /**
   * Set up credentials interactively
   * @returns Promise that resolves when setup is complete
   */
  public async setupCredentials(): Promise<void> {
    const { complete, missingConfigs } = this.checkCredentials();

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

      let validContent = false;
      let content = "";

      while (!validContent) {
        content = await question(`\nPaste your ${config.name}: `);
        
        if (!content.trim()) {
          console.log("Please enter a valid credential.");
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
        const filePath = path.join(this.projectRoot, config.filename);
        try {
          // Ensure directory exists
          const dir = path.dirname(filePath);
          if (!fs.existsSync(dir)) {
            fs.mkdirSync(dir, { recursive: true });
          }
          
          fs.writeFileSync(filePath, content.trim());
          console.log(`✅ Successfully saved ${config.name}`);
          validContent = true;
        } catch (error) {
          console.error(`Error writing file: ${error instanceof Error ? error.message : error}`);
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
    return path.join(this.projectRoot, filename);
  }

  /**
   * Get credential content by name
   * @param name Name of the credential
   * @returns The credential content as string
   */
  public getCredentialByName(name: string): string | null {
    const config = this.configs.find(c => c.name === name);
    if (!config) return null;
    
    const filePath = this.getCredentialPath(config.filename);
    if (!fs.existsSync(filePath)) return null;
    
    return fs.readFileSync(filePath, 'utf-8').trim();
  }

  /**
   * Initialize the credentials manager - ensure creds directory exists
   */
  public async init(): Promise<void> {
    // Create creds directory if it doesn't exist
    const credsDir = path.join(this.projectRoot, 'creds');
    if (!fs.existsSync(credsDir)) {
      fs.mkdirSync(credsDir, { recursive: true });
    }
  }
}
