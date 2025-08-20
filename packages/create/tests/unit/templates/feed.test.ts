import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createFeedTemplate } from '../../../src/templates/feed';
import { createTemplate } from '../../../src/templates/base';
import { ProjectPlan } from '../../../src/types';

vi.mock('../../../src/templates/base');

const mockCreateTemplate = createTemplate as any;

describe('createFeedTemplate', () => {
  const mockPlan: ProjectPlan = {
    projectName: 'social-feed-app',
    description: 'A social feed application',
    platform: 'social-feed',
  };

  beforeEach(() => {
    vi.clearAllMocks();
    mockCreateTemplate.mockResolvedValue(undefined);
  });

  it('should call createTemplate with correct social feed configuration', async () => {
    const projectPath = '/test/social-feed-app';
    const projectName = 'social-feed-app';
    const templatePath = '/templates/social-feed';

    await createFeedTemplate(projectPath, projectName, templatePath, mockPlan);

    expect(mockCreateTemplate).toHaveBeenCalledWith(
      projectPath,
      projectName,
      templatePath,
      mockPlan,
      expect.objectContaining({
        type: 'social-feed',
        displayName: 'Social Feed',
        successMessage: '🎉 Your social feed is ready for vibe coding! 🎉',
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
    const projectName = 'my-feed-app';
    const templatePath = '/custom/template';
    const customPlan: ProjectPlan = {
      projectName: 'my-feed-app',
      description: 'Custom social feed',
      platform: 'social-feed',
      workerDependencies: ['worker1', 'worker2'],
    };

    await createFeedTemplate(
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
    mockCreateTemplate.mockRejectedValue(
      new Error('Feed template creation failed')
    );

    await expect(
      createFeedTemplate('/test/path', 'test-app', '/template', mockPlan)
    ).rejects.toThrow('Feed template creation failed');
  });
});
