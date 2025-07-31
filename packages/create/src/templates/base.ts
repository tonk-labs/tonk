import fs from 'fs-extra';
import path from 'path';
import ora from 'ora';
import chalk from 'chalk';
import { ProjectPlan, TemplateType } from '../types';

export interface TemplateConfig {
  type: TemplateType;
  displayName: string;
  successMessage: string;
  nextSteps: Array<{
    command: string;
    description: string;
  }>;
  customizeProject?: (
    projectPath: string,
    projectName: string,
    plan: ProjectPlan
  ) => Promise<void>;
}

export async function createTemplate(
  projectPath: string,
  projectName: string,
  templatePath: string,
  plan: ProjectPlan,
  config: TemplateConfig
) {
  const spinner = ora(
    `Creating ${config.displayName} project structure...\n`
  ).start();

  try {
    // List template contents before copying
    await fs.readdir(templatePath);

    await fs.copy(templatePath, projectPath, {
      filter: (src: string) => {
        // Get the relative path from the template directory
        const relativePath = path.relative(templatePath, src);
        // Only filter out node_modules and .git within the template
        const shouldCopy = !relativePath
          .split(path.sep)
          .some(part => part === 'node_modules' || part === '.git');
        if (!shouldCopy) {
          console.log(`Skipping: ${relativePath}`);
        }
        return shouldCopy;
      },
      overwrite: true,
      errorOnExist: false,
    });

    // Verify project contents after copying
    await fs.readdir(projectPath);

    // Update package.json name
    const packageJsonPath = path.join(projectPath, 'package.json');
    if (await fs.pathExists(packageJsonPath)) {
      const packageJson = await fs.readJson(packageJsonPath);
      packageJson.name = projectName;
      await fs.writeJson(packageJsonPath, packageJson, { spaces: 2 });
    }

    // Run custom project setup if provided
    if (config.customizeProject) {
      spinner.text = 'Customizing project...';
      await config.customizeProject(projectPath, projectName, plan);
    }

    // Create tonk.config.json with project plan
    const tonkConfig: {
      name: string;
      plan: ProjectPlan;
      template: string;
    } = {
      name: projectName,
      plan,
      template: config.type,
    };

    await fs.writeJSON(path.join(projectPath, 'tonk.config.json'), tonkConfig, {
      spaces: 2,
    });

    spinner.succeed(`${config.displayName} project created successfully!`);

    // Install dependencies
    spinner.start('Installing dependencies...');
    const { execSync } = await import('child_process');
    process.chdir(projectPath);
    execSync('pnpm install', { stdio: 'inherit' });
    spinner.succeed('Dependencies installed successfully!');

    // Print next steps instructions
    console.log('\n' + chalk.bold(config.successMessage));
    console.log('\n' + chalk.bold('Next:'));

    // Add navigation step
    console.log(
      '  • ' +
        chalk.cyan(`cd ${projectName}`) +
        ' - Navigate to your new project'
    );

    // Add custom next steps
    config.nextSteps.forEach(step => {
      console.log('  • ' + chalk.cyan(step.command) + ' - ' + step.description);
    });

    console.log('  • Open your favourite vibe coding editor to get started.\n');
  } catch (error) {
    spinner.fail(`Failed to create ${config.displayName} project`);
    console.error(error);
    throw error;
  }
}
