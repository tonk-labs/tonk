import { describe, it, expect, vi, beforeEach } from 'vitest';
import fs from 'fs-extra';
import { execSync } from 'child_process';
import { createTemplate, TemplateConfig } from '../../../src/templates/base';
import { ProjectPlan } from '../../../src/types';

vi.mock('fs-extra');
vi.mock('child_process');
vi.mock('ora');

const mockFs = fs as any;
const mockExecSync = execSync as any;

describe('createTemplate', () => {
  const mockPlan: ProjectPlan = {
    projectName: 'test-project',
    description: 'Test description',
    platform: 'react',
  };

  const mockConfig: TemplateConfig = {
    type: 'react',
    displayName: 'React',
    successMessage: '🎉 Your React app is ready!',
    nextSteps: [
      { command: 'pnpm dev', description: 'Start development server' },
      { command: 'pnpm build', description: 'Build for production' },
    ],
  };

  beforeEach(() => {
    vi.clearAllMocks();

    // Mock fs methods
    mockFs.readdir.mockResolvedValue(['file1.js', 'file2.js']);
    mockFs.copy.mockResolvedValue(undefined);
    mockFs.pathExists.mockResolvedValue(true);
    mockFs.readJson.mockResolvedValue({ name: 'old-name', version: '1.0.0' });
    mockFs.writeJson.mockResolvedValue(undefined);
    mockFs.writeJSON.mockResolvedValue(undefined);

    // Mock execSync
    mockExecSync.mockReturnValue('');

    // Mock process.chdir
    process.chdir = vi.fn();
  });

  it('should create template successfully', async () => {
    const projectPath = '/test/path/my-project';
    const projectName = 'my-project';
    const templatePath = '/templates/react';

    await createTemplate(
      projectPath,
      projectName,
      templatePath,
      mockPlan,
      mockConfig
    );

    // Verify template directory reading
    expect(mockFs.readdir).toHaveBeenCalledWith(templatePath);

    // Verify template copying
    expect(mockFs.copy).toHaveBeenCalledWith(
      templatePath,
      projectPath,
      expect.objectContaining({
        overwrite: true,
        errorOnExist: false,
      })
    );

    // Verify project directory reading after copying
    expect(mockFs.readdir).toHaveBeenCalledWith(projectPath);

    // Verify package.json update
    expect(mockFs.readJson).toHaveBeenCalledWith(
      expect.stringContaining('package.json')
    );
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
        template: mockConfig.type,
      }),
      { spaces: 2 }
    );

    // Verify dependency installation
    expect(process.chdir).toHaveBeenCalledWith(projectPath);
    expect(mockExecSync).toHaveBeenCalledWith('pnpm install', {
      stdio: 'inherit',
    });
  });

  it('should call custom project setup when provided', async () => {
    const customizeProject = vi.fn().mockResolvedValue(undefined);
    const configWithCustomization: TemplateConfig = {
      ...mockConfig,
      customizeProject,
    };

    await createTemplate(
      '/test/path',
      'test-project',
      '/templates/react',
      mockPlan,
      configWithCustomization
    );

    expect(customizeProject).toHaveBeenCalledWith(
      '/test/path',
      'test-project',
      mockPlan
    );
  });

  it('should filter out node_modules and .git directories', async () => {
    await createTemplate(
      '/test/path',
      'test-project',
      '/templates/react',
      mockPlan,
      mockConfig
    );

    expect(mockFs.copy).toHaveBeenCalledWith(
      expect.any(String),
      expect.any(String),
      expect.objectContaining({
        filter: expect.any(Function),
      })
    );

    // Test the filter function
    const copyCall = mockFs.copy.mock.calls[0];
    const filterFunction = copyCall[2].filter;

    // Should filter out node_modules
    expect(filterFunction('/templates/react/node_modules')).toBe(false);
    expect(filterFunction('/templates/react/some/node_modules/package')).toBe(
      false
    );

    // Should filter out .git
    expect(filterFunction('/templates/react/.git')).toBe(false);
    expect(filterFunction('/templates/react/some/.git/config')).toBe(false);

    // Should allow normal files
    expect(filterFunction('/templates/react/src/index.js')).toBe(true);
    expect(filterFunction('/templates/react/package.json')).toBe(true);
  });

  it('should handle missing package.json gracefully', async () => {
    mockFs.pathExists.mockResolvedValue(false);

    await expect(
      createTemplate(
        '/test/path',
        'test-project',
        '/templates/react',
        mockPlan,
        mockConfig
      )
    ).resolves.not.toThrow();

    // Should not attempt to read/write package.json if it doesn't exist
    expect(mockFs.readJson).not.toHaveBeenCalled();
    expect(mockFs.writeJson).not.toHaveBeenCalled();
  });

  it('should handle template creation errors', async () => {
    mockFs.copy.mockRejectedValue(new Error('Copy failed'));

    await expect(
      createTemplate(
        '/test/path',
        'test-project',
        '/templates/react',
        mockPlan,
        mockConfig
      )
    ).rejects.toThrow('Copy failed');
  });

  it('should handle dependency installation errors', async () => {
    mockExecSync.mockImplementation(() => {
      throw new Error('pnpm install failed');
    });

    await expect(
      createTemplate(
        '/test/path',
        'test-project',
        '/templates/react',
        mockPlan,
        mockConfig
      )
    ).rejects.toThrow('pnpm install failed');
  });
});
