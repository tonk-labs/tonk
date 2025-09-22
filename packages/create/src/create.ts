import chalk from 'chalk';
import { Command } from 'commander';
import fs from 'fs-extra';
import inquirer from 'inquirer';
import ora from 'ora';
import path from 'path';
import process from 'process';
import { fileURLToPath } from 'url';

/**
 * Gets available templates by scanning the templates directory
 * @returns Array of template names
 */
async function getAvailableTemplates(): Promise<string[]> {
  try {
    // Get the directory where this script is running from
    const moduleUrl = import.meta.url;
    const moduleDirPath = path.dirname(fileURLToPath(moduleUrl));

    // Templates are in the package root, so go up from src or dist
    const templatesPath = path.resolve(moduleDirPath, '..', 'templates');

    const items = await fs.readdir(templatesPath, { withFileTypes: true });
    return items
      .filter(item => item.isDirectory())
      .map(item => item.name)
      .sort();
  } catch (error) {
    console.error('Error reading templates directory:', error);
    return [];
  }
}

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
    const localPath = path.resolve(moduleDirPath, '..', relativePath);

    if (await fs.pathExists(localPath)) {
      return localPath;
    } else {
      // If local path doesn't exist, try global node_modules
      const { execSync } = await import('child_process');
      const result = execSync('npm root -g');
      const globalNodeModules = result ? result.toString().trim() : '';

      // Look for the package in global node_modules
      const globalPath = path.join(
        globalNodeModules,
        '@tonk/create',
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
let packageJson: any;

try {
  const packageJsonPath = await resolvePackagePath('package.json');
  packageJson = JSON.parse(fs.readFileSync(packageJsonPath, 'utf8'));
} catch (error) {
  console.error('Error resolving package.json:', error);
  if (!process.env.VITEST) {
    process.exit(1);
  }
  // In test environment, use mock data
  packageJson = { version: '0.0.0-test' };
}

const program = new Command();

// Function to create project structure
export async function createProject(
  projectName: string,
  templateName: string,
  _projectPath: string | null = null
) {
  const spinner = ora(`Creating ${templateName} project...`).start();
  let projectPath = _projectPath;

  try {
    // Check if pnpm is installed, if not, install it
    const { execSync } = await import('child_process');
    try {
      execSync('pnpm --version', { stdio: 'pipe' });
    } catch {
      spinner.text = 'Installing pnpm...';
      execSync('npm install -g pnpm', { stdio: 'inherit' });
      spinner.text = `Creating ${templateName} project...`;
    }

    // Create project directory
    projectPath = projectPath || path.resolve(projectName);
    await fs.ensureDir(projectPath);

    // Find template path
    const moduleUrl = import.meta.url;
    const moduleDirPath = path.dirname(fileURLToPath(moduleUrl));
    const templatePath = path.resolve(
      moduleDirPath,
      '..',
      'templates',
      templateName
    );

    // Ensure templatePath is defined before using it
    if (!templatePath || !(await fs.pathExists(templatePath))) {
      throw new Error(
        `Template path not found for "${templateName}": ${templatePath}`
      );
    }

    // Copy template files
    spinner.text = 'Copying template files...';
    await fs.copy(templatePath, projectPath, {
      filter: (src: string) => {
        // Get the relative path from the template directory
        const relativePath = path.relative(templatePath, src);
        // Only filter out node_modules and .git within the template
        return !relativePath
          .split(path.sep)
          .some(part => part === 'node_modules' || part === '.git');
      },
      overwrite: true,
      errorOnExist: false,
    });

    // Update package.json name
    const packageJsonPath = path.join(projectPath, 'package.json');
    if (await fs.pathExists(packageJsonPath)) {
      const packageJson = await fs.readJson(packageJsonPath);
      packageJson.name = projectName;
      await fs.writeJson(packageJsonPath, packageJson, { spaces: 2 });
    }

    spinner.succeed(`${templateName} project created successfully!`);

    // Install dependencies
    spinner.start('Installing dependencies...');
    process.chdir(projectPath);
    execSync('pnpm install', { stdio: 'inherit' });
    spinner.succeed('Dependencies installed successfully!');

    // Print success message and next steps
    console.log(
      '\n' + chalk.bold(`🎉 Your ${templateName} project is ready! 🎉`)
    );
    console.log('\n' + chalk.bold('Next:'));
    console.log(
      '  • ' +
        chalk.cyan(`cd ${projectName}`) +
        ' - Navigate to your new project'
    );
    console.log(
      '  • ' + chalk.cyan('pnpm dev') + ' - Start the development server'
    );
    console.log('  • Open your favourite editor to get started.\n');
  } catch (error) {
    spinner.fail('Failed to setup project');
    console.error(error);
    if (!process.env.VITEST) {
      process.exit(1);
    }
    throw error;
  }
}

const createApp = async (options: {
  init: boolean;
  template?: string;
  name?: string;
}) => {
  try {
    console.log('Scaffolding tonk code...');

    // Get available templates
    const availableTemplates = await getAvailableTemplates();

    if (availableTemplates.length === 0) {
      throw new Error('No templates found in templates directory');
    }

    let templateName = options.template;
    let projectName = options.name;

    // If template not provided, prompt for selection
    if (!templateName) {
      const templateAnswer = await inquirer.prompt([
        {
          type: 'list',
          name: 'template',
          message: 'Select a template:',
          choices: availableTemplates,
        },
      ]);
      templateName = templateAnswer.template;
    }

    // Validate template exists
    if (!templateName || !availableTemplates.includes(templateName)) {
      throw new Error(
        `Template "${templateName}" not found. Available templates: ${availableTemplates.join(', ')}`
      );
    }

    // If project name not provided, prompt for it
    if (!projectName && !options.init) {
      const nameAnswer = await inquirer.prompt([
        {
          type: 'input',
          name: 'projectName',
          message: 'Project name:',
          default: 'my-new-tonk',
        },
      ]);
      projectName = nameAnswer.projectName;
    }

    // Determine final project name and path
    const finalProjectName = options.init
      ? path.basename(process.cwd())
      : projectName || 'my-new-tonk';

    const projectPath = options.init ? process.cwd() : null;

    // TypeScript now knows templateName is defined due to the validation above
    await createProject(finalProjectName, templateName as string, projectPath);
  } catch (error) {
    console.error(chalk.red('Error:'), error);
    if (!process.env.VITEST) {
      process.exit(1);
    }
    throw error;
  }
};

program
  .name('create')
  .description('Scaffold code for your Tonk projects')
  .version(packageJson.version, '-v, --version', 'Output the current version')
  .option('-i, --init', 'initialize in the current folder')
  .option('-t, --template <type>', 'template type (e.g., react, node)')
  .option('-n, --name <name>', 'project name')
  .action(async options => {
    try {
      console.log(chalk.bold('\nWelcome to Tonk! 🚀\n'));
      await createApp({
        init: options.init ?? false,
        template: options.template,
        name: options.name,
      });
    } catch (error) {
      console.error(chalk.red('CLI Error:'), error);
      if (!process.env.VITEST) {
        process.exit(1);
      }
    }
  });

// Only run CLI when not in test environment
if (!process.env.VITEST && !process.env.NODE_ENV?.includes('test')) {
  program.parse(process.argv);
}
