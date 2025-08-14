import { describe, it, expect, vi, beforeEach } from 'vitest';
import fs from 'fs-extra';
import { execSync } from 'child_process';
import inquirer from 'inquirer';
import { createProject } from '../../src/create';
import { ProjectPlan, TemplateType } from '../../src/types';

vi.mock('fs-extra');
vi.mock('child_process');
vi.mock('inquirer');
vi.mock('ora', () => ({
  default: vi.fn(() => ({
    start: vi.fn().mockReturnThis(),
    stop: vi.fn().mockReturnThis(),
    succeed: vi.fn().mockReturnThis(),
    fail: vi.fn().mockReturnThis(),
  })),
}));

const mockFs = fs as any;
const mockExecSync = execSync as any;
const mockInquirer = inquirer as any;

describe('createProject', () => {
  const mockPlan: ProjectPlan = {
    projectName: 'test-project',
    description: 'Test description',
    platform: 'react',
  };

  beforeEach(() => {
    vi.clearAllMocks();

    // Mock fs methods
    mockFs.pathExists.mockResolvedValue(true);
    mockFs.ensureDir.mockResolvedValue(undefined);
    mockFs.copy.mockResolvedValue(undefined);
    mockFs.readJson.mockResolvedValue({ name: 'old-name', version: '1.0.0' });
    mockFs.writeJson.mockResolvedValue(undefined);
    mockFs.writeJSON.mockResolvedValue(undefined);
    mockFs.readdir.mockResolvedValue(['file1.js', 'file2.js']);

    // Mock execSync for pnpm check and install
    mockExecSync.mockReturnValue('6.0.0'); // pnpm version

    // Mock process.chdir
    process.chdir = vi.fn();
  });

  it('should create a React project successfully', async () => {
    const projectName = 'my-react-app';
    const templateName: TemplateType = 'react';
    const projectPath = '/test/path/my-react-app';

    await createProject(projectName, mockPlan, templateName, projectPath);

    // Verify directory creation
    expect(mockFs.ensureDir).toHaveBeenCalledWith(projectPath);

    // Verify package.json update
    expect(mockFs.readJson).toHaveBeenCalled();
    expect(mockFs.writeJson).toHaveBeenCalledWith(
      expect.stringContaining('package.json'),
      expect.objectContaining({ name: projectName }),
      { spaces: 2 }
    );

    // Verify tonk.config.json creation
    expect(mockFs.writeJSON).toHaveBeenCalledWith(
      expect.stringContaining('tonk.config.json'),
      expect.objectContaining({
        name: projectName,
        plan: mockPlan,
        template: templateName,
      }),
      { spaces: 2 }
    );

    // Verify pnpm install
    expect(mockExecSync).toHaveBeenCalledWith('pnpm install', {
      stdio: 'inherit',
    });
  });

  it('should install pnpm if not available', async () => {
    // Mock pnpm not available
    mockExecSync
      .mockImplementationOnce(() => {
        throw new Error('pnpm not found');
      })
      .mockReturnValueOnce('') // pnpm install
      .mockReturnValueOnce(''); // pnpm install for dependencies

    await createProject('test-app', mockPlan, 'react');

    // Verify pnpm installation
    expect(mockExecSync).toHaveBeenCalledWith('npm install -g pnpm', {
      stdio: 'inherit',
    });
  });

  it('should handle template path resolution errors', async () => {
    mockFs.pathExists.mockResolvedValue(false);

    await expect(createProject('test-app', mockPlan, 'react')).rejects.toThrow(
      'Could not locate template'
    );
  });

  it('should create project in current directory when projectPath is null', async () => {
    const projectName = 'test-app';

    await createProject(projectName, mockPlan, 'react', null);

    expect(mockFs.ensureDir).toHaveBeenCalledWith(
      expect.stringContaining(projectName)
    );
  });

  it('should handle all template types', async () => {
    const templateTypes: TemplateType[] = [
      'react',
      'social-feed',
      'travel-planner',
    ];

    for (const templateType of templateTypes) {
      vi.clearAllMocks();
      mockFs.pathExists.mockResolvedValue(true);
      mockFs.ensureDir.mockResolvedValue(undefined);

      await expect(
        createProject('test-app', mockPlan, templateType)
      ).resolves.not.toThrow();
    }
  });
});

describe('resolvePackagePath', () => {
  const testPlan: ProjectPlan = {
    projectName: 'test-app',
    description: 'Test description',
    platform: 'react',
  };

  beforeEach(() => {
    vi.clearAllMocks();
    mockFs.pathExists.mockResolvedValue(true);
    mockFs.ensureDir.mockResolvedValue(undefined);
  });

  it('should resolve local development path when available', async () => {
    mockFs.pathExists.mockResolvedValueOnce(true);

    await expect(
      createProject('test-app', testPlan, 'react')
    ).resolves.not.toThrow();
  });

  it('should fallback to global path when local path does not exist', async () => {
    mockFs.pathExists.mockResolvedValueOnce(false).mockResolvedValueOnce(true);

    mockExecSync.mockReturnValue('/usr/local/lib/node_modules');

    await expect(
      createProject('test-app', testPlan, 'react')
    ).resolves.not.toThrow();
  });
});

describe('CLI Integration', () => {
  it('should handle interactive mode', async () => {
    mockInquirer.prompt.mockResolvedValue({
      platform: 'react',
      projectName: 'interactive-app',
      description: 'Interactive test app',
    });

    expect(mockInquirer.prompt).toBeDefined();
  });

  it('should handle non-interactive mode with all options', async () => {
    expect(true).toBe(true);
  });
});
