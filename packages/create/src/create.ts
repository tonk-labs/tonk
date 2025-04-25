import chalk from "chalk";
import { Command } from "commander";
import fs from "fs-extra";
import inquirer from "inquirer";
import ora from "ora";
import path from "path";
import process from "process";
import { fileURLToPath } from "url";
import { createReactTemplate } from "./templates/react";
import { createNodeTemplate } from "./templates/node";
import { ProjectPlan, TemplateType } from "./types";

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
        "@tonk/create",
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
    type: "list",
    name: "platform",
    message: "What template would you like to use?",
    choices: ["react", "node"],
    default: "react",
  },
  {
    type: "input",
    name: "projectName",
    message: "What is your project named?",
    default: "my-tonk-app",
  },
  {
    type: "input",
    name: "description",
    message: "Briefly describe your project:",
  },
];

// Function to create project structure
export async function createProject(
  projectName: string,
  plan: ProjectPlan,
  templateName: TemplateType,
  _projectPath: string | null = null
) {
  const spinner = ora("Creating project structure...").start();
  let projectPath = _projectPath;

  try {
    // Create project directory
    projectPath = projectPath || path.resolve(projectName);
    await fs.ensureDir(projectPath);

    // Find template path
    let templatePath;

    try {
      templatePath = await resolvePackagePath(`templates/${templateName}`);
    } catch (error) {
      console.error(
        `Error resolving template path for "${templateName}":`,
        error
      );
      throw new Error(
        `Could not locate template "${templateName}". Please ensure the package is installed correctly and the template exists.`
      );
    }

    // Ensure templatePath is defined before using it
    if (!templatePath || !(await fs.pathExists(templatePath))) {
      throw new Error(
        `Template path not found for "${templateName}": ${templatePath}`
      );
    }

    // Switch on template type and call appropriate template creator
    switch (templateName) {
      case "react": {
        await createReactTemplate(projectPath, projectName, templatePath, plan);
        break;
      }
      case "node": {
        await createNodeTemplate(projectPath, projectName, templatePath, plan);
        break;
      }
    }
  } catch (error) {
    spinner.fail("Failed to setup project");
    console.error(error);
    process.exit(1);
  }
  spinner.stop();
}

const createApp = async (options: { init: boolean }) => {
  try {
    console.log("Scaffolding tonk code...");
    // Prepare questions, removing projectName question if provided as argument
    const questions = options.init
      ? [...projectQuestions.slice(1)]
      : projectQuestions;

    // Get project details
    const answers = await inquirer.prompt(questions);

    // Generate project plan
    const plan = answers;

    // Create project with generated plan and template
    const finalProjectName = options.init
      ? path.basename(process.cwd())
      : answers.projectName || "my-tonk-app";

    const templateName = answers.platform as TemplateType;
    let projectPath = options.init ? process.cwd() : null;
    await createProject(finalProjectName, plan, templateName, projectPath);

    console.log("🎉 Tonk project ready for vibe coding!");
  } catch (error) {
    console.error(chalk.red("Error:"), error);
    process.exit(1);
  }
};

program
  .name("create")
  .description("Scaffold code for your Tonk projects")
  .version(packageJson.version, "-v, --version", "Output the current version")
  .option("-i, --init", "initialize in the folder")
  .action(async (options) => {
    console.log(chalk.bold("\nWelcome to Tonk! 🚀\n"));
    await createApp({ init: options.init ?? false });
    return;
  });

program.parse(process.argv);
