import fs from "fs-extra";
import path from "path";
import ora from "ora";
import chalk from "chalk";
import { ProjectPlan } from "../types";

export async function createWorkerTemplate(
  projectPath: string,
  projectName: string,
  templatePath: string,
  plan: ProjectPlan,
) {
  const spinner = ora("Creating Worker project structure...").start();

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

    spinner.text = "Updating project files...";
    
    // Use existing project name and description from the plan
    const workerName = projectName;
    const description = plan.description || "A Tonk worker service";
    const port = "5555"; // Default port

    // Update package.json
    const packageJsonPath = path.join(projectPath, "package.json");
    if (await fs.pathExists(packageJsonPath)) {
      const packageJson = await fs.readJson(packageJsonPath);
      packageJson.name = workerName;
      packageJson.description = description;
      packageJson.bin = {
        [workerName]: "dist/cli.js",
      };
      await fs.writeJson(packageJsonPath, packageJson, { spaces: 2 });
    }

    // Create worker.config.js with the port
    const workerConfigPath = path.join(projectPath, "worker.config.js");
    if (await fs.pathExists(workerConfigPath)) {
      let workerConfigContent = await fs.readFile(workerConfigPath, "utf8");
      workerConfigContent = workerConfigContent.replace(
        /\{\{port\}\}/g,
        port,
      );
      await fs.writeFile(workerConfigPath, workerConfigContent);
    }

    // Update CLI file
    const cliPath = path.join(projectPath, "src", "cli.ts");
    if (await fs.pathExists(cliPath)) {
      let cliContent = await fs.readFile(cliPath, "utf8");
      cliContent = cliContent.replace(/\{\{name\}\}/g, workerName);
      cliContent = cliContent.replace(/\{\{description\}\}/g, description);
      cliContent = cliContent.replace(/\{\{version\}\}/g, "1.0.0");
      await fs.writeFile(cliPath, cliContent);
    }

    // Update index.ts to replace the hello endpoint message
    const indexPath = path.join(projectPath, "src", "index.ts");
    if (await fs.pathExists(indexPath)) {
      let indexContent = await fs.readFile(indexPath, "utf8");
      indexContent = indexContent.replace(/\{\{name\}\}/g, workerName);
      await fs.writeFile(indexPath, indexContent);
    }

    // Create tonk.config.json with project plan
    await fs.writeJSON(
      path.join(projectPath, "tonk.config.json"),
      {
        name: projectName,
        plan,
        template: "worker",
      },
      { spaces: 2 },
    );

    spinner.succeed("Worker project created successfully!");

    // Install dependencies
    spinner.start("Installing dependencies...");
    const { execSync } = await import("child_process");
    process.chdir(projectPath);
    execSync("pnpm install", { stdio: "inherit" });
    spinner.succeed("Dependencies installed successfully!");

    // Print next steps instructions
    console.log(
      "\n" + chalk.bold("🎉 Your Tonk worker is ready for vibe coding! 🎉"),
    );
    console.log("\n" + chalk.bold("Next:"));
    console.log("  • Open your favorite vibe coding editor to begin coding.\n");
    console.log(
      "  • " +
        chalk.cyan("pnpm dev") +
        " - Start the worker in development mode",
    );
    console.log(
      "  • " + chalk.cyan("pnpm build") + " - Build your worker for production",
    );
    console.log(
      "  • " +
        chalk.cyan("tonk worker register") +
        " - Register your worker with Tonk\n",
    );
  } catch (error) {
    spinner.fail("Failed to create Worker project");
    console.error(error);
    throw error;
  }
}
