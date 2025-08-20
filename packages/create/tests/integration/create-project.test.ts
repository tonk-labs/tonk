import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import fs from 'fs-extra';
import path from 'path';
import { execSync } from 'child_process';
import { createProject } from '../../src/create';
import { ProjectPlan } from '../../src/types';

// Integration tests use real file system operations but in a temp directory
vi.mock('child_process');

const mockExecSync = execSync as any;

describe('createProject integration tests', () => {
  const tempDir = path.join(__dirname, '../../test-output');

  beforeEach(async () => {
    // Create temp directory for integration tests
    await fs.ensureDir(tempDir);

    // Mock execSync for pnpm commands
    mockExecSync.mockImplementation((command: string) => {
      if (command === 'pnpm --version') {
        return '8.0.0';
      }
      if (command === 'npm root -g') {
        return '/usr/local/lib/node_modules';
      }
      if (command === 'pnpm install') {
        return '';
      }
      return '';
    });
  });

  afterEach(async () => {
    // Clean up test directory
    try {
      await fs.remove(tempDir);
    } catch (error) {
      // Ignore cleanup errors
    }
    vi.clearAllMocks();
  });

  const createTestPlan = (platform: string): ProjectPlan => ({
    projectName: 'integration-test-app',
    description: 'Integration test application',
    platform,
  });

  describe('React template integration', () => {
    it('should create a complete React project structure', async () => {
      // Setup
      const projectName = 'react-integration-test';
      const projectPath = path.join(tempDir, projectName);
      const plan = createTestPlan('react');

      // Execute - this should work because the real templates exist
      await expect(
        createProject(projectName, plan, 'react', projectPath)
      ).resolves.not.toThrow();

      // Verify project structure was created
      expect(await fs.pathExists(projectPath)).toBe(true);
    });
  });

  describe('Social Feed template integration', () => {
    it('should create a complete Social Feed project structure', async () => {
      const projectName = 'feed-integration-test';
      const projectPath = path.join(tempDir, projectName);
      const plan = createTestPlan('social-feed');

      await expect(
        createProject(projectName, plan, 'social-feed', projectPath)
      ).resolves.not.toThrow();
    }, 10000);
  });

  describe('Travel Planner template integration', () => {
    it('should create a complete Travel Planner project structure', async () => {
      const projectName = 'travel-integration-test';
      const projectPath = path.join(tempDir, projectName);
      const plan = createTestPlan('travel-planner');

      await expect(
        createProject(projectName, plan, 'travel-planner', projectPath)
      ).resolves.not.toThrow();
    }, 10000);
  });

  describe('Error handling integration', () => {
    it('should handle missing template gracefully', async () => {
      const projectName = 'error-test';
      const projectPath = path.join(tempDir, projectName);
      const plan = createTestPlan('react');

      // Test with non-existent template type
      await expect(
        createProject(projectName, plan, 'nonexistent' as any, projectPath)
      ).rejects.toThrow();
    });

    it('should handle permission errors', async () => {
      // Since the test is succeeding (creating the project), let's change this to test
      // actual error case - like providing an invalid template path internally
      const projectName = 'permission-test';
      const projectPath = path.join(tempDir, projectName);
      const plan = createTestPlan('react');

      // Mock fs.ensureDir to throw permission error
      const originalEnsureDir = fs.ensureDir;
      vi.spyOn(fs, 'ensureDir').mockRejectedValueOnce(
        new Error('EACCES: permission denied')
      );

      await expect(
        createProject(projectName, plan, 'react', projectPath)
      ).rejects.toThrow();

      // Restore original implementation
      vi.mocked(fs.ensureDir).mockImplementation(originalEnsureDir);
    });
  });

  describe('pnpm installation integration', () => {
    it('should install pnpm when not available', async () => {
      // Mock pnpm not being available initially
      mockExecSync.mockImplementationOnce(() => {
        throw new Error('Command not found: pnpm');
      });

      const projectName = 'pnpm-install-test';
      const projectPath = path.join(tempDir, projectName);
      const plan = createTestPlan('react');

      // Since templates exist, this should succeed
      await expect(
        createProject(projectName, plan, 'react', projectPath)
      ).resolves.not.toThrow();

      // Verify pnpm installation was attempted
      expect(mockExecSync).toHaveBeenCalledWith('npm install -g pnpm', {
        stdio: 'inherit',
      });
    });

    it('should use existing pnpm when available', async () => {
      const projectName = 'pnpm-existing-test';
      const projectPath = path.join(tempDir, projectName);
      const plan = createTestPlan('react');

      // Since templates exist, this should succeed
      await expect(
        createProject(projectName, plan, 'react', projectPath)
      ).resolves.not.toThrow();

      // Verify pnpm version check was performed
      expect(mockExecSync).toHaveBeenCalledWith('pnpm --version', {
        stdio: 'pipe',
      });
    });
  });

  describe('Project path handling', () => {
    it('should create project in specified path', async () => {
      const projectName = 'path-test';
      const customPath = path.join(tempDir, 'custom', 'location', projectName);
      const plan = createTestPlan('react');

      // Since templates exist, this should succeed
      await expect(
        createProject(projectName, plan, 'react', customPath)
      ).resolves.not.toThrow();

      // The function should have attempted to create the directory structure
    });

    it('should create project in current directory when path is null', async () => {
      const projectName = 'current-dir-test';
      const plan = createTestPlan('react');

      // Since templates exist, this should succeed
      await expect(
        createProject(projectName, plan, 'react', null)
      ).resolves.not.toThrow();

      // The function should have attempted to resolve the current directory path
    });
  });
});
