import fs from "fs-extra";
import path from "path";
import ora from "ora";
import chalk from "chalk";
import { ProjectPlan } from "../types";

export async function createReactTemplate(
  projectPath: string,
  projectName: string,
  templatePath: string,
  plan: ProjectPlan & { useWorkers?: boolean; workers?: string[] },
) {
  const spinner = ora("Creating React project structure...").start();

  try {
    // Copy template files
    console.log(`Copying from: ${templatePath}`);
    console.log(`Copying to: ${projectPath}`);

    // List template contents before copying
    const templateContents = await fs.readdir(templatePath);
    console.log(`Template directory contains: ${templateContents.join(", ")}`);

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
      `Project directory now contains: ${projectContents.join(", ")}`,
    );

    // Update package.json name
    const packageJsonPath = path.join(projectPath, "package.json");
    if (await fs.pathExists(packageJsonPath)) {
      const packageJson = await fs.readJson(packageJsonPath);
      packageJson.name = projectName;
      await fs.writeJson(packageJsonPath, packageJson, { spaces: 2 });
    }

    // Create tonk.config.json with project plan
    const tonkConfig: {
      name: string;
      plan: ProjectPlan & { useWorkers?: boolean; workers?: string[] };
      template: string;
      workers?: string[];
    } = {
      name: projectName,
      plan,
      template: "react",
    };

    // Add workers if specified
    if (plan.useWorkers && plan.workers && plan.workers.length > 0) {
      tonkConfig.workers = plan.workers;
    }

    await fs.writeJSON(path.join(projectPath, "tonk.config.json"), tonkConfig, {
      spaces: 2,
    });

    spinner.succeed("React project created successfully!");

    // Install dependencies
    spinner.start("Installing dependencies...");
    const { execSync } = await import("child_process");
    process.chdir(projectPath);
    execSync("pnpm install", { stdio: "inherit" });
    spinner.succeed("Dependencies installed successfully!");

    // Install and start workers if specified
    if (plan.useWorkers && plan.workers && plan.workers.length > 0) {
      spinner.start("Installing worker dependencies...");
      for (const worker of plan.workers) {
        console.log(`\nInstalling worker: ${worker}`);
        execSync(`tonk worker install ${worker}`, { stdio: "inherit" });
      }
      spinner.succeed("Worker dependencies installed successfully!");
    }

    // Print next steps instructions
    console.log(
      "\n" + chalk.bold("🎉 Your Tonk react app is ready for vibe coding! 🎉"),
    );
    console.log("\n" + chalk.bold("Next:"));
    console.log("  • Open your favorite vibe coding editor to begin coding.\n");
    console.log(
      "  • " + chalk.cyan("pnpm dev") + " - Start the development server",
    );

    console.log(
      "  • " +
        chalk.cyan("pnpm build") +
        " - Build your project for production.\n",
    );
  } catch (error) {
    spinner.fail("Failed to create React project");
    console.error(error);
    throw error;
  }
}
