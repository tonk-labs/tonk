import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createTravelTemplate } from '../../../src/templates/travel';
import { createTemplate } from '../../../src/templates/base';
import { ProjectPlan } from '../../../src/types';

vi.mock('../../../src/templates/base');

const mockCreateTemplate = createTemplate as any;

describe('createTravelTemplate', () => {
  const mockPlan: ProjectPlan = {
    projectName: 'travel-planner-app',
    description: 'A travel planning application',
    platform: 'travel-planner',
  };

  beforeEach(() => {
    vi.clearAllMocks();
    mockCreateTemplate.mockResolvedValue(undefined);
  });

  it('should call createTemplate with correct travel planner configuration', async () => {
    const projectPath = '/test/travel-planner-app';
    const projectName = 'travel-planner-app';
    const templatePath = '/templates/travel-planner';

    await createTravelTemplate(
      projectPath,
      projectName,
      templatePath,
      mockPlan
    );

    expect(mockCreateTemplate).toHaveBeenCalledWith(
      projectPath,
      projectName,
      templatePath,
      mockPlan,
      expect.objectContaining({
        type: 'travel-planner',
        displayName: 'Travel Planner',
        successMessage: '🎉 Your travel planner is ready for vibe coding! 🎉',
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
    const projectName = 'my-travel-app';
    const templatePath = '/custom/template';
    const customPlan: ProjectPlan = {
      projectName: 'my-travel-app',
      description: 'Custom travel planner',
      platform: 'travel-planner',
      implementationLog: ['Initialize project', 'Setup routing'],
    };

    await createTravelTemplate(
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
      new Error('Travel template creation failed')
    );

    await expect(
      createTravelTemplate('/test/path', 'test-app', '/template', mockPlan)
    ).rejects.toThrow('Travel template creation failed');
  });
});
