#!/usr/bin/env node
import { Command } from "commander";
import inquirer from "inquirer";
import chalk from "chalk";
import ora from "ora";
import fs from "fs-extra";
import path from "path";
import { fileURLToPath } from "url";

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
    type: "checkbox",
    name: "features",
    message: "Select the features you need:",
    choices: ["File Storage", "State Backup", "Multiplayer"],
  },
  {
    type: "input",
    name: "pages",
    message: "List the main pages you want (comma-separated):",
    filter: (input: string) =>
      input.split(",").map((page: string) => page.trim()),
  },
  {
    type: "input",
    name: "description",
    message: "Briefly describe your project and its main functionality:",
  },
];

// Function to generate project plan using LLM
interface ProjectAnswers {
  projectType: string;
  features: string[];
  pages: string[];
  description: string;
}

export async function generateProjectPlan(answers: ProjectAnswers) {
  const prompt = `You are an expert in full stack development and local-first tooling. Based on the following project requirements, generate a structured implementation plan.
    Prioritize keepsync and WebSocket for local-first development (info found in src/lib/keepsync/), and Tailwind for styling.
    
    Project Type: ${answers.projectType}
    Features: ${answers.features.join(", ")}
    Pages: ${answers.pages.join(", ")}
    Description: ${answers.description}

  //   Provide a response in this exact JSON format:
  //   {
  //     "components": [{ name: string, description: string }],
  //     "views": [{ name: string, description: string }],
  //     "state": [{ name: string, schema: object }]
  //     "modules": [{ name: string, description: string }]
  //   }

  // Components are like the the pure UI pieces for our design system (e.g. button, navbar, dropdown, card)
  // Views are the main pages of the application, so they describe parts of the application (e.g. landing, login, home, compose)
  // State describes what should be the stateful pieces of the project (e.g. user profile, post listings, messages)
  // Modules describe units of functionality that don't sit inside views, components or state (e.g. googleAPI, mathUtils, crypto)

  //   Keep the response focused and practical. Include only essential components and libraries.`;

  try {
    const response = await fetch("http://localhost:11434/api/generate", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        model: "deepseek-r1:8b",
        prompt,
        stream: false,
        format: "json",
      }),
    });

    if (!response.ok) {
      throw new Error(`Ollama request failed: ${response.statusText}`);
    }

    interface OllamaResponse {
      response: string;
      context?: number[];
      created_at: string;
      done: boolean;
      model: string;
      total_duration?: number;
    }

    const data = (await response.json()) as OllamaResponse;
    const planJson = JSON.parse(data.response);

    //   // the users original high-level description of the project
    //   planJson.projectDescription = answers.description;

    // Validate the response structure
    return {
      ...planJson,
      description: answers.description,
    };
  } catch (error) {
    console.error("Error generating project plan:", error);
    // Fallback to a basic plan if LLM fails
    return {
      description: answers.description,
      components: [],
      views: [{ name: "Home", description: "A basic home page" }],
      state: [],
      modules: [],
    };
  }
}

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
      // For ESM, get the directory name using import.meta.url
      const moduleUrl = import.meta.url;
      const moduleDirPath = path.dirname(fileURLToPath(moduleUrl));

      // Try local development path (one directory up from current file)
      const localPath = path.resolve(moduleDirPath, "../templates/default");

      if (await fs.pathExists(localPath)) {
        templatePath = localPath;
      } else {
        // If local path doesn't exist, try global node_modules
        const { execSync } = await import("child_process");
        const globalNodeModules = execSync("npm root -g").toString().trim();

        // Look for the package in global node_modules
        const globalPath = path.join(
          globalNodeModules,
          "@tonk/create-tinyfoot-app/templates/default"
        );

        if (await fs.pathExists(globalPath)) {
          templatePath = globalPath;
        } else {
          throw new Error(
            "Could not locate template directory in local or global paths"
          );
        }
      }
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
  .argument("[project-directory]", "Project directory")
  .action(async (projectDirectory: string | undefined) => {
    console.log(chalk.bold("\nWelcome to Tinyfoot! 🚀\n"));

    try {
      // Get project details
      const answers = await inquirer.prompt(projectQuestions);

      // Generate project plan
      const spinner = ora("Generating project plan...").start();
      const plan = await generateProjectPlan(answers);
      spinner.succeed("Project plan generated!");

      // Create project with generated plan
      const finalProjectName = projectDirectory || answers.projectName;
      await createProject(finalProjectName, plan);
    } catch (error) {
      console.error(chalk.red("Error:"), error);
      process.exit(1);
    }
  });

program.parse(process.argv);
