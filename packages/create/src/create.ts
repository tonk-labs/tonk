import { Command } from "commander";
import inquirer from "inquirer";
import chalk from "chalk";
import ora from "ora";
import fs from "fs-extra";
import path from "path";
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
    type: "input",
    name: "projectName",
    message: "What is your project named?",
    default: "my-tonk-app",
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
    type: "list",
    name: "platform",
    message: "What platform do you want to use?",
    choices: ["react"],
  },
  {
    type: "input",
    name: "description",
    message: "Briefly describe your project and its main functionality:",
  },
];

// Function to create project structure
export async function createProject(
  projectName: string,
  plan: ProjectPlan,
  _templateName: TemplateType,
) {
  const spinner = ora("Creating project structure...").start();
  let projectPath = "";

  try {
    // Create project directory
    projectPath = path.resolve(projectName);
    await fs.ensureDir(projectPath);

    // Find template path
    let templatePath;
    let templateName = _templateName === "default" ? "react" : _templateName;

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
    try {
      switch (templateName) {
        case "react":
          await createReactTemplate(
            projectPath,
            projectName,
            templatePath,
            plan,
          );
          break;

        default:
          await createReactTemplate(
            projectPath,
            projectName,
            templatePath,
            plan,
          );
          break;
      }
    } catch (error) {
      spinner.fail(`Failed to create ${templateName} project`);
      console.error(error);
      process.exit(1);
    }
  } catch (error) {
    spinner.fail("Failed to setup project");
    console.error(error);
    process.exit(1);
  }
  spinner.stop();
}

program
  .name("create-app")
  .description("Create a new Tonk app")
  .version(packageJson.version, "-v, --version", "Output the current version")
  .argument("[project-name]", "Name of the project to create")
  .action(async (projectNameArg) => {
    console.log(chalk.bold("\nTonk! 🚀\n"));

    try {
      // Prepare questions, removing projectName question if provided as argument
      const questions = [...projectQuestions];
      if (projectNameArg) {
        // Remove the projectName question if name was provided as argument
        const projectNameIndex = questions.findIndex(
          (q) => q.name === "projectName",
        );
        if (projectNameIndex !== -1) {
          questions.splice(projectNameIndex, 1);
        }
      }

      // Get project details
      const answers = await inquirer.prompt(questions);
      const options = program.opts();

      // If project name was provided as argument, add it to answers
      if (projectNameArg) {
        answers.projectName = projectNameArg;
      }

      // Generate project plan
      const plan = answers;

      // Create project with generated plan and template
      const finalProjectName =
        projectNameArg || options.name || answers.projectName || "my-tonk-app";
      const templateName = answers.platform as TemplateType;
      await createProject(finalProjectName, plan, templateName);
    } catch (error) {
      console.error(chalk.red("Error:"), error);
      process.exit(1);
    }
  });

program.parse(process.argv);
