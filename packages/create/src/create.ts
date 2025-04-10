import { Command } from "commander";
import inquirer from "inquirer";
import chalk from "chalk";
import ora from "ora";
import fs from "fs-extra";
import path from "path";
import process from "process";
import { fileURLToPath } from "url";
import { ProjectPlan, TemplateType } from "./types";
import { createReactTemplate } from "./templates/react";

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
  _templateName: TemplateType,
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
    const templateName = "react";

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
    await createReactTemplate(projectPath, projectName, templatePath, plan);
  } catch (error) {
    spinner.fail("Failed to setup project");
    console.error(error);
    process.exit(1);
  }
  spinner.stop();
}

const createApp = async (init: boolean) => {
  try {
    console.log("Scaffolding tonk code...");
    // Prepare questions, removing projectName question if provided as argument
    const questions = init
      ? [...projectQuestions.slice(1)]
      : [...projectQuestions];

    // Get project details
    const answers = await inquirer.prompt(questions);
    const options = program.opts();

    // Generate project plan
    const plan = answers;

    // Create project with generated plan and template
    const finalProjectName = init
      ? path.basename(process.cwd())
      : options.name || answers.projectName || "my-tonk-app";
    const templateName = answers.platform as TemplateType;
    let projectPath = init ? process.cwd() : null;
    await createProject(finalProjectName, plan, templateName, projectPath);

    console.log("🎉 Tonk project ready for vibe coding!");
  } catch (error) {
    console.error(chalk.red("Error:"), error);
    process.exit(1);
  }
};

const createTemplate = async () => {
  try {
    console.log(chalk.italic("Scaffolding tonk integration..."));

    // Questions for integration template
    const integrationQuestions = [
      {
        type: "input",
        name: "projectName",
        message: "What is your integration named?",
        default: "my-tonk-integration",
      },
      {
        type: "input",
        name: "description",
        message:
          "Briefly describe your integration and what data it will handle:",
      },
    ];

    // Get integration details
    const answers = await inquirer.prompt(integrationQuestions);
    const options = program.opts();

    // Generate project plan
    const plan = {
      projectDescription: answers.description,
      implementationLog: [],
    };

    // Create project with generated plan and template
    const finalProjectName =
      options.name || answers.projectName || "my-tonk-integration";
    await createProject(finalProjectName, plan, "integration");
  } catch (error) {
    console.error(chalk.red("Error:"), error);
    process.exit(1);
  }
};

const TEMPLATE_TYPES = ["app", "integration"];
const TEMPLATE_DESCRIPTION = [
  "Creates an empty Tonk app",
  "Creates a new Tonk integration for importing or fetching data",
];
program
  .name("create")
  .description("Scaffold code for your Tonk projects")
  .version(packageJson.version, "-v, --version", "Output the current version")
  .option("-i, --init", "initialize in the folder")
  .argument("[type]", "Type of template to scaffold")
  .exitOverride((e) => {
    if (e.message.includes("invalid for argument 'type'")) {
      console.log("\n");
      program.outputHelp();
      console.log("\n\n");
      process.exit(8);
    } else {
      throw e;
    }
  })
  .action(async (typeArg, options) => {
    console.log(chalk.bold("\nWelcome to Tonk! 🚀\n"));

    switch (typeArg) {
      case TEMPLATE_TYPES[0]: {
        await createApp(options.init);
        return;
      }
      case TEMPLATE_TYPES[1] || `${TEMPLATE_TYPES[1]}s`: {
        await createTemplate();
        return;
      }
      default: {
        console.log(
          `Hmm, I don't recognize the template type of '${typeArg}'.`
        );
        console.log("\n");
        console.log(`Available types:`);
        TEMPLATE_TYPES.forEach((ttype, i) =>
          console.log(` ${ttype}: \t\t${TEMPLATE_DESCRIPTION[i]}`)
        );
        console.log("\n\n");
      }
    }
  });

program.parse(process.argv);
