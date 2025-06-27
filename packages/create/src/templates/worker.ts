import fs from "fs-extra";
import path from "path";
import { ProjectPlan } from "../types";
import { createTemplate, TemplateConfig } from "./base";

async function customizeWorkerProject(
  projectPath: string,
  projectName: string,
  plan: ProjectPlan,
) {
  // Use existing project name and description from the plan
  const workerName = projectName;
  const description = plan.description || "A Tonk worker service";
  const port = "5555"; // Default port

  // Update package.json with worker-specific fields
  const packageJsonPath = path.join(projectPath, "package.json");
  if (await fs.pathExists(packageJsonPath)) {
    const packageJson = await fs.readJson(packageJsonPath);
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
    workerConfigContent = workerConfigContent.replace(/\{\{port\}\}/g, port);
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
}

export async function createWorkerTemplate(
  projectPath: string,
  projectName: string,
  templatePath: string,
  plan: ProjectPlan,
) {
  const config: TemplateConfig = {
    type: "worker",
    displayName: "Worker",
    successMessage: "🎉 Your Tonk worker is ready for vibe coding! 🎉",
    nextSteps: [
      {
        command: "pnpm dev",
        description: "Start the worker in development mode",
      },
      {
        command: "pnpm build",
        description: "Build your worker for production",
      },
      {
        command: "tonk worker register",
        description: "Register your worker with Tonk",
      },
    ],
    customizeProject: customizeWorkerProject,
  };

  await createTemplate(projectPath, projectName, templatePath, plan, config);
}
