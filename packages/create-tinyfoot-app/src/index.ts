#!/usr/bin/env node
import { Command } from "commander";
import inquirer from "inquirer";
import chalk from "chalk";
import ora from "ora";
import fs from "fs-extra";
import path from "path";
import { fileURLToPath } from "url";

/**
 * Resolves a package path by checking both local development and global installation paths
 * @param relativePath Path relative to the package root
 * @returns Resolved absolute path to the requested file/directory
 */
async function resolvePackagePath(relativePath: string): Promise<string> {
  try {
    // For ESM, get the directory name using import.meta.url
    const moduleUrl = import.meta.url;
    const moduleDirPath = path.dirname(fileURLToPath(moduleUrl));

    // Try local development path first
    const localPath = path.resolve(moduleDirPath, "..", relativePath);

    if (await fs.pathExists(localPath)) {
      return localPath;
    } else {
      // If local path doesn't exist, try global node_modules
      const { execSync } = await import("child_process");
      const globalNodeModules = execSync("npm root -g").toString().trim();

      // Look for the package in global node_modules
      const globalPath = path.join(
        globalNodeModules,
        "@tonk/create-tinyfoot-app",
        relativePath
      );

      if (await fs.pathExists(globalPath)) {
        return globalPath;
      } else {
        throw new Error(
          `Could not locate ${relativePath} in local or global paths`
        );
      }
    }
  } catch (error) {
    console.error(`Error resolving path ${relativePath}:`, error);
    throw error;
  }
}

// Get package.json for version information
let packageJson;

try {
  const packageJsonPath = await resolvePackagePath("package.json");
  packageJson = JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));
} catch (error) {
  console.error("Error resolving package.json:", error);
  process.exit(1);
}

const program = new Command();

// Questions to understand project requirements
const projectQuestions = [
  {
    type: "input",
    name: "projectName",
    message: "What is your project named?",
    default: "my-tinyfoot-app",
  },
  {
    type: "list",
    name: "projectType",
    message: "What type of project are you building?",
    choices: [
      "Productivity System",
      "Creative Tool",
      "Professional Services",
      "Community Space",
      "Learning & Education",
      "Other",
    ],
  },
  {
    type: "input",
    name: "description",
    message: "Briefly describe your project and its main functionality:",
  },
];

// Function to generate project plan using LLM
// interface ProjectAnswers {
//   projectType: string;
//   features: string[];
//   pages: string[];
//   description: string;
// }

// Function to create project structure
interface ProjectPlan {
  implementationLog: string[];
  projectDescription: string;
}

export async function createProject(projectName: string, plan: ProjectPlan) {
  const spinner = ora("Creating project structure...").start();
  let projectPath = "";

  try {
    // Create project directory
    projectPath = path.resolve(projectName);
    await fs.ensureDir(projectPath);

    // Find template path - simplified approach
    let templatePath;

    try {
      // Use the existing resolvePackagePath function to find the template directory
      templatePath = await resolvePackagePath("templates/default");
    } catch (error) {
      console.error("Error resolving template path:", error);
      throw new Error(
        "Could not locate template directory. Please ensure the package is installed correctly."
      );
    }

    // Ensure templatePath is defined before using it
    if (!templatePath || !(await fs.pathExists(templatePath))) {
      throw new Error(`Template path not found: ${templatePath}`);
    }

    console.log(`Using template from: ${templatePath}`);

    // Copy template files
    try {
      console.log(`Copying from: ${templatePath}`);
      console.log(`Copying to: ${projectPath}`);

      // List template contents before copying
      const templateContents = await fs.readdir(templatePath);
      console.log(
        `Template directory contains: ${templateContents.join(", ")}`
      );

      await fs.copy(templatePath, projectPath, {
        filter: (src: string) => {
          // Get the relative path from the template directory
          const relativePath = path.relative(templatePath, src);
          // Only filter out node_modules and .git within the template
          const shouldCopy = !relativePath
            .split(path.sep)
            .some((part) => part === "node_modules" || part === ".git");
          if (!shouldCopy) {
            console.log(`Skipping: ${relativePath}`);
          }
          return shouldCopy;
        },
        overwrite: true,
        errorOnExist: false,
      });

      // Verify project contents after copying
      const projectContents = await fs.readdir(projectPath);
      console.log(
        `Project directory now contains: ${projectContents.join(", ")}`
      );
    } catch (copyError: any) {
      console.error("Error during template copying:", copyError);
      throw new Error(`Failed to copy template files: ${copyError.message}`);
    }

    // Update package.json name
    const packageJsonPath = path.join(projectPath, "package.json");
    if (await fs.pathExists(packageJsonPath)) {
      const packageJson = await fs.readJson(packageJsonPath);
      packageJson.name = projectName;
      await fs.writeJson(packageJsonPath, packageJson, { spaces: 2 });
    }

    // Create tinyfoot.config.json with project plan
    await fs.writeJSON(
      path.join(projectPath, "tinyfoot.config.json"),
      {
        name: projectName,
        plan,
      },
      { spaces: 2 }
    );

    spinner.succeed("Project created successfully!");
  } catch (error) {
    spinner.fail("Failed to create project");
    console.error(error);
    process.exit(1);
  }

  // Execute post-creation commands in a separate try-catch
  try {
    // Change directory into the project
    process.chdir(projectPath);

    // Install dependencies
    spinner.start("Installing dependencies...");
    const { execSync } = await import("child_process");
    execSync("npm install", { stdio: "inherit" });
    spinner.succeed("Dependencies installed successfully!");

    // Print next steps instructions
    console.log("\n" + chalk.bold("🎉 Your Tinyfoot app is ready! 🎉"));
    console.log("\n" + chalk.bold("Next steps:"));
    console.log(
      "  • " + chalk.cyan("npm run dev") + " - Start the development server"
    );
    console.log(
      "  • You may launch claude code or any other AI editor in this directory to begin coding.\n"
    );

    // Don't unref the process - we want the parent to wait for the child
  } catch (error) {
    spinner.fail(
      "Failed to run post-creation commands, please run them manually in the project folder. (See top-level README.md)"
    );
    console.error(error);
    process.exit(1);
  }
}

program
  .name("create-tinyfoot-app")
  .description("Create a new Tinyfoot application")
  .version(packageJson.version, "-v, --version", "Output the current version")
  .option("-n, --name <project-name>", "Project Name")
  .action(async () => {
    console.log(chalk.bold("\nWelcome to Tinyfoot! 🚀\n"));

    try {
      // Get project details
      const answers = await inquirer.prompt(projectQuestions);
      const options = program.opts();

      // Generate project plan
      // const spinner = ora("Generating project plan...").start();
      const plan = answers;
      // spinner.succeed("Project plan generated!");

      // Create project with generated plan
      const finalProjectName =
        options.name || answers.projectName || "my-tinyfoot-app";
      await createProject(finalProjectName, plan);
    } catch (error) {
      console.error(chalk.red("Error:"), error);
      process.exit(1);
    }
  });

program.parse(process.argv);
