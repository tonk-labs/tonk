import fs from "fs-extra";
import path from "path";
import ora from "ora";
import chalk from "chalk";
import { ProjectPlan } from "../types";

export async function createWorkspaceTemplate(
  projectPath: string,
  projectName: string,
  templatePath: string,
  plan: ProjectPlan,
) {
  const spinner = ora("Creating workspace structure...\n").start();

  try {
    // List template contents before copying
    const templateContents = await fs.readdir(templatePath);

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

    // Create tonk.config.json at the root level with project plan
    const tonkConfig: {
      name: string;
      plan: ProjectPlan;
      template: string;
    } = {
      name: projectName,
      plan,
      template: "workspace",
    };

    await fs.writeJSON(path.join(projectPath, "tonk.config.json"), tonkConfig, {
      spaces: 2,
    });

    spinner.succeed("Workspace created successfully!");

    // Print next steps instructions
    console.log("\n" + chalk.bold("🎉 Your Tonk workspace is ready! 🎉"));
    console.log("\n" + chalk.bold("Next:"));
    console.log(
      "  • " +
        chalk.cyan(`cd ${projectName}`) +
        " - Navigate to your new workspace",
    );
    console.log(
      "  • " +
        chalk.cyan("cd console && pnpm install") +
        " - Install console dependencies",
    );
    console.log(
      "  • " +
        chalk.cyan("cd console && pnpm dev") +
        " - Start the console app",
    );
    console.log("  • Open your favourite vibe coding editor to get started.\n");
  } catch (error) {
    spinner.fail("Failed to create workspace");
    console.error(error);
    throw error;
  }
}

