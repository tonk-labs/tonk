import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createReactTemplate } from '../../../src/templates/react';
import { createTemplate } from '../../../src/templates/base';
import { ProjectPlan } from '../../../src/types';

vi.mock('../../../src/templates/base');

const mockCreateTemplate = createTemplate as any;

describe('createReactTemplate', () => {
  const mockPlan: ProjectPlan = {
    projectName: 'react-app',
    description: 'A React application',
    platform: 'react',
  };

  beforeEach(() => {
    vi.clearAllMocks();
    mockCreateTemplate.mockResolvedValue(undefined);
  });

  it('should call createTemplate with correct React configuration', async () => {
    const projectPath = '/test/react-app';
    const projectName = 'react-app';
    const templatePath = '/templates/react';

    await createReactTemplate(projectPath, projectName, templatePath, mockPlan);

    expect(mockCreateTemplate).toHaveBeenCalledWith(
      projectPath,
      projectName,
      templatePath,
      mockPlan,
      expect.objectContaining({
        type: 'react',
        displayName: 'React',
        successMessage: '🎉 Your Tonk react app is ready for vibe coding! 🎉',
        nextSteps: expect.arrayContaining([
          expect.objectContaining({
            command: 'pnpm dev',
            description: 'Start the development server',
          }),
          expect.objectContaining({
            command: 'pnpm build',
            description: 'Build your project for production',
          }),
        ]),
      })
    );
  });

  it('should pass through all parameters correctly', async () => {
    const projectPath = '/custom/path';
    const projectName = 'my-custom-app';
    const templatePath = '/custom/template';
    const customPlan: ProjectPlan = {
      projectName: 'my-custom-app',
      description: 'Custom description',
      platform: 'react',
      implementationLog: ['Step 1', 'Step 2'],
    };

    await createReactTemplate(
      projectPath,
      projectName,
      templatePath,
      customPlan
    );

    expect(mockCreateTemplate).toHaveBeenCalledWith(
      projectPath,
      projectName,
      templatePath,
      customPlan,
      expect.any(Object)
    );
  });

  it('should handle errors from createTemplate', async () => {
    mockCreateTemplate.mockRejectedValue(new Error('Template creation failed'));

    await expect(
      createReactTemplate('/test/path', 'test-app', '/template', mockPlan)
    ).rejects.toThrow('Template creation failed');
  });
});
