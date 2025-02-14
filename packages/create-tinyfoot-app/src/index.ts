#!/usr/bin/env node
import { Command } from 'commander';
import inquirer from 'inquirer';
import chalk from 'chalk';
import ora from 'ora';
import fs from 'fs-extra';
import path from 'path';

const program = new Command();

// Questions to understand project requirements
const projectQuestions = [
  {
    type: 'input',
    name: 'projectName',
    message: 'What is your project named?',
    default: 'my-tinyfoot-app'
  },
  {
    type: 'list',
    name: 'projectType',
    message: 'What type of project are you building?',
    choices: [
      'Productivity System',
      'Creative Tool',
      'Professional Services',
      'Community Space',
      'Learning & Education',
      'Other'
    ]
  },
  {
    type: 'checkbox',
    name: 'features',
    message: 'Select the features you need:',
    choices: [
      'Authentication',
      'Database',
      'File Storage',
      'API Integration',
      'Real-time Updates',
    ]
  },
  {
    type: 'input',
    name: 'pages',
    message: 'List the main pages you want (comma-separated):',
    filter: (input: string) => input.split(',').map((page: string) => page.trim())
  },
  {
    type: 'input',
    name: 'description',
    message: 'Briefly describe your project and its main functionality:',
  }
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
    Prioritize Yjs and Redis for local-first, Prisma and Sqlite for database management, and Tailwind for styling.
    
    Project Type: ${answers.projectType}
    Features: ${answers.features.join(', ')}
    Pages: ${answers.pages.join(', ')}
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
    const response = await fetch('http://localhost:11434/api/generate', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        model: 'deepseek-r1:8b',
        prompt,
        stream: false,
        format: 'json'
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

    const data = await response.json() as OllamaResponse;
    const planJson = JSON.parse(data.response);

    // Validate the response structure
    if (!planJson.components || !planJson.dataModel ||
      !planJson.implementationSteps || !planJson.recommendedLibraries) {
      throw new Error('Invalid response structure from LLM');
    }

    return planJson;
  } catch (error) {
    console.error('Error generating project plan:', error);
    // Fallback to a basic plan if LLM fails
    return {
      components: [
        { name: 'Layout', description: 'Main layout wrapper' },
        { name: 'Navigation', description: 'Site navigation' }
      ],
      dataModel: {},
      implementationSteps: [
        'Initialize project structure',
        'Set up routing',
        'Implement authentication'
      ],
      recommendedLibraries: [
        { name: 'next-auth', purpose: 'Authentication' },
        { name: 'prisma', purpose: 'Database ORM' }
      ]
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
  const spinner = ora('Creating project structure...').start();

  try {
    // Create project directory
    const projectPath = path.resolve(projectName);
    await fs.ensureDir(projectPath);

    // Copy template files
    // TODO: Implement template copying based on project type

    // Create tinyfoot.config.json with project plan
    await fs.writeJSON(
      path.join(projectPath, 'tinyfoot.config.json'),
      {
        name: projectName,
        plan
      },
      { spaces: 2 }
    );

    spinner.succeed('Project created successfully!');

    console.log('\n' + chalk.green('Next steps:'));
    console.log(`  cd ${projectName}`);
    console.log('  npm install');
    console.log('  npm run dev\n');

  } catch (error) {
    spinner.fail('Failed to create project');
    console.error(error);
    process.exit(1);
  }
}

program
  .name('create-tinyfoot-app')
  .description('Create a new Tinyfoot application')
  .argument('[project-directory]', 'Project directory')
  .action(async (projectDirectory: string | undefined) => {
    console.log(chalk.bold('\nWelcome to Tinyfoot! 🚀\n'));

    try {
      // Get project details
      const answers = await inquirer.prompt(projectQuestions);

      // Generate project plan
      const spinner = ora('Generating project plan...').start();
      const plan = await generateProjectPlan(answers);
      spinner.succeed('Project plan generated!');

      // Create project with generated plan
      const finalProjectName = projectDirectory || answers.projectName;
      await createProject(finalProjectName, plan);

    } catch (error) {
      console.error(chalk.red('Error:'), error);
      process.exit(1);
    }
  });

program.parse(process.argv);
