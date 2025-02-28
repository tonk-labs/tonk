#!/usr/bin/env node
import { Command } from "commander";
import inquirer from "inquirer";
import chalk from "chalk";
import ora from "ora";
import fs from "fs-extra";
import path from "path";
import open from "open";
import { spawn } from "child_process";

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
    choices: [
      "Authentication",
      "Database",
      "File Storage",
      "API Integration",
      "Real-time Updates",
    ],
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

    Provide a response in this exact JSON format:
    {
      "components": [{ "name": "string", "description": "string" }],
      "dataModel": { /* relevant data model structure */ },
      "implementationSteps": ["string"],
      "recommendedLibraries": [{ "name": "string", "purpose": "string" }]
    }

    Keep the response focused and practical. Include only essential components and libraries.`;

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

    // the users original high-level description of the project
    planJson.projectDescription = answers.description;

    // Validate the response structure
    if (
      !planJson.components ||
      !planJson.dataModel ||
      !planJson.implementationSteps ||
      !planJson.recommendedLibraries
    ) {
      throw new Error("Invalid response structure from LLM");
    }

    return planJson;
  } catch (error) {
    console.error("Error generating project plan:", error);
    // Fallback to a basic plan if LLM fails
    return {
      components: [
        { name: "Layout", description: "Main layout wrapper" },
        { name: "Navigation", description: "Site navigation" },
      ],
      dataModel: {},
      projectDescription: answers.description,
      implementationSteps: [
        "Initialize project structure",
        "Set up routing",
        "Implement authentication",
      ],
      recommendedLibraries: [
        { name: "next-auth", purpose: "Authentication" },
        { name: "prisma", purpose: "Database ORM" },
      ],
    };
  }
}

// Function to create project structure
interface ProjectPlan {
  components: Array<{ name: string; description: string }>;
  dataModel: Record<string, unknown>;
  implementationSteps: string[];
  recommendedLibraries: Array<{ name: string; purpose: string }>;
}

export async function createProject(projectName: string, plan: ProjectPlan) {
  const spinner = ora("Creating project structure...").start();
  let projectPath = "";

  try {
    // Create project directory
    projectPath = path.resolve(projectName);
    await fs.ensureDir(projectPath);

    // Copy template files from default template
    // TODO: copy based on project type
    const templatePath = path.join(
      // When running from npm exec, we need to use fileURLToPath
      new URL("../templates/default", import.meta.url).pathname,
    );
    if (await fs.pathExists(templatePath)) {
      await fs.copy(templatePath, projectPath, {
        filter: (src: string) => {
          // Skip node_modules and .git if they exist in template
          return !src.includes("node_modules") && !src.includes(".git");
        },
      });

      // Update package.json name
      const packageJsonPath = path.join(projectPath, "package.json");
      if (await fs.pathExists(packageJsonPath)) {
        const packageJson = await fs.readJson(packageJsonPath);
        packageJson.name = projectName;
        await fs.writeJson(packageJsonPath, packageJson, { spaces: 2 });
      }
    } else {
      console.error(
        chalk.red(`Error: Template directory not found at ${templatePath}`),
      );
      throw new Error("Template directory not found");
    }

    // Create tinyfoot.config.json with project plan
    await fs.writeJSON(
      path.join(projectPath, "tinyfoot.config.json"),
      {
        name: projectName,
        plan,
      },
      { spaces: 2 },
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

    // Start the development server
    spinner.start("Starting development server...");
    const devProcess = spawn("npm", ["run", "dev"], {
      stdio: ["inherit", "pipe", "inherit"],
    });

    // Store the process PID for cleanup
    const devProcessPid = devProcess.pid;

    // Wait for the dev server to be ready
    devProcess.stdout?.on("data", async (data) => {
      const output = data.toString();
      process.stdout.write(output); // Show the output in real time

      if (output.includes("compiled successfully")) {
        spinner.succeed("Development server is ready!");
        spinner.start("Opening website...");

        // Add a small delay before opening the browser
        setTimeout(async () => {
          try {
            await open("http://localhost:3000");
            spinner.succeed("Website opened in your default browser!");
            console.log(
              chalk.yellow(
                "\nPress Ctrl+C to stop the development server and exit.",
              ),
            );
          } catch (err) {
            spinner.fail("Failed to open browser");
            console.error("Error opening browser:", err);
          }
        }, 1000);
      }
    });

    // Create a clean shutdown function
    const cleanShutdown = () => {
      try {
        console.log(chalk.yellow("\nTerminating development server..."));

        // Try multiple ways to kill the process to ensure it's terminated
        if (devProcess && !devProcess.killed) {
          // First try to kill it gracefully
          devProcess.kill("SIGTERM");

          // If we have the PID, also try process group kill for safety
          if (devProcessPid) {
            try {
              // For Unix/Mac systems, kill the process group
              process.kill(-devProcessPid, "SIGTERM");
            } catch (e) {
              // Ignore errors from this attempt
            }
          }
        }
      } catch (error) {
        console.error("Error shutting down dev server:", error);
      }
      process.exit(0);
    };

    // Add process termination handlers to properly clean up the child process
    process.on("SIGINT", cleanShutdown);
    process.on("SIGTERM", cleanShutdown);

    // Handle process completion
    devProcess.on("close", (code) => {
      if (code !== 0) {
        spinner.fail(`Development server process exited with code ${code}`);
      }
    });

    // Handle process errors
    devProcess.on("error", (error) => {
      spinner.fail("Failed to start development server");
      console.error(error);
      process.exit(1);
    });

    // Don't unref the process - we want the parent to wait for the child
  } catch (error) {
    spinner.fail(
      "Failed to run post-creation commands, please run them manually in the project folder. (See top-level README.md)",
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
