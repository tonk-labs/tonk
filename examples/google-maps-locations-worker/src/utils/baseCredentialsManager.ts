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
          console.log(`\nEnter the content for ${config.filename} (type 'END' on a new line when finished):`);
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
          const filePath = await question("\nEnter the path to the existing file: ");
          try {
            content = fs.readFileSync(filePath, "utf-8");
          } catch (error) {
            console.error(`Error reading file: ${error.message}`);
            continue;
          }
        } else if (option === "3") {
          console.log(`\nSkipping ${config.name}. You'll need to add this file manually later.`);
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
        const filePath = path.join(this.projectRoot, config.filename);
        try {
          fs.writeFileSync(filePath, content);
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
    return path.join(this.projectRoot, filename);
  }
}
