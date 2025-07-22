import chalk from "chalk";
import { Command } from "commander";
import fs from "fs-extra";
import inquirer from "inquirer";
import ora from "ora";
import path from "path";
import process from "process";
import { fileURLToPath } from "url";
import { createReactTemplate } from "./templates/react";
import { createFeedTemplate } from "./templates/feed";
import { ProjectPlan, TemplateType } from "./types";
import { createTravelTemplate } from "./templates/travel";

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
        relativePath,
      );

      if (await fs.pathExists(globalPath)) {
        return globalPath;
      } else {
        throw new Error(
          `Could not locate ${relativePath} in local or global paths`,
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
    message: "What type of project would you like to create?",
    choices: [
      {
        name: "React - blank Tonk app",
        value: "react",
      },
      {
        name: "Social Feed - share and comment on posts",
        value: "social-feed",
      },
      {
        name: "Travel Planner - plan trips with friends",
        value: "travel-planner",
      },
    ],
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
  _projectPath: string | null = null,
) {
  const spinner = ora("Creating project structure...").start();
  let projectPath = _projectPath;

  try {
    // Check if pnpm is installed, if not, install it
    const { execSync } = await import("child_process");
    try {
      execSync("pnpm --version", { stdio: "pipe" });
    } catch (error) {
      spinner.text = "Installing pnpm...";
      execSync("npm install -g pnpm", { stdio: "inherit" });
      spinner.text = "Creating project structure...";
    }

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
        error,
      );
      throw new Error(
        `Could not locate template "${templateName}". Please ensure the package is installed correctly and the template exists.`,
      );
    }

    // Ensure templatePath is defined before using it
    if (!templatePath || !(await fs.pathExists(templatePath))) {
      throw new Error(
        `Template path not found for "${templateName}": ${templatePath}`,
      );
    }

    // Switch on template type and call appropriate template creator
    switch (templateName) {
      case "react": {
        await createReactTemplate(projectPath, projectName, templatePath, plan);
        break;
      }
      case "social-feed": {
        await createFeedTemplate(projectPath, projectName, templatePath, plan);
        break;
      }
      case "travel-planner": {
        await createTravelTemplate(
          projectPath,
          projectName,
          templatePath,
          plan,
        );
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

const createApp = async (options: {
  init: boolean;
  template?: TemplateType;
  name?: string;
  description?: string;
}) => {
  try {
    console.log("Scaffolding tonk code...");

    let answers: any;

    // Check if all required options are provided for non-interactive mode
    const isNonInteractive =
      options.template && (!options.init || options.name);

    if (isNonInteractive) {
      // Non-interactive mode - use provided options
      answers = {
        platform: options.template,
        projectName: options.name || "my-tonk-app",
        description: options.description || "",
      };
    } else {
      // Interactive mode - prompt for missing information
      const questions = options.init
        ? [...projectQuestions.slice(1)]
        : projectQuestions;

      // Filter out questions for options that were already provided
      const filteredQuestions = questions.filter((q) => {
        if (q.name === "platform" && options.template) return false;
        if (q.name === "projectName" && options.name) return false;
        if (q.name === "description" && options.description) return false;
        return true;
      });

      const promptAnswers = await inquirer.prompt(filteredQuestions);

      // Merge provided options with prompted answers
      answers = {
        platform: options.template || promptAnswers.platform,
        projectName: options.name || promptAnswers.projectName,
        description: options.description || promptAnswers.description,
      };
    }

    // Generate project plan
    const plan = answers;

    // Create project with generated plan and template
    const finalProjectName = options.init
      ? path.basename(process.cwd())
      : answers.projectName || "my-tonk-app";

    const templateName = answers.platform as TemplateType;
    let projectPath = options.init ? process.cwd() : null;
    await createProject(finalProjectName, plan, templateName, projectPath);
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
  .option(
    "-t, --template <type>",
    "template type (react, social feed, travel planner)",
  )
  .option("-n, --name <name>", "project name")
  .option("-d, --description <description>", "project description")
  .action(async (options) => {
    console.log(chalk.bold("\nWelcome to Tonk! 🚀\n"));
    await createApp({
      init: options.init ?? false,
      template: options.template,
      name: options.name,
      description: options.description,
    });
    return;
  });

program.parse(process.argv);
