import { describe, it, expect, beforeEach, vi, beforeAll } from 'vitest';
import { generateProjectPlan } from '../index';

describe('Implementation Plan Generator', () => {
  // Reset mocks before each test
  beforeEach(() => {
    vi.clearAllMocks();
  });

  // Check if Ollama is running before running tests
  beforeAll(async () => {
    try {
      const health = await fetch('http://localhost:11434');
      if (!health.ok) {
        throw new Error('Ollama server is not running');
      }
    } catch (error) {
      throw new Error('Please start Ollama server before running tests');
    }
  });

  it(
    'should generate a valid implementation plan',
    async () => {
      const mockAnswers = {
        projectType: 'web',
        features: ['authentication', 'real-time-sync'],
        pages: ['home', 'dashboard'],
        description: 'A collaborative note-taking app',
      };

      const plan = await generateProjectPlan(mockAnswers);

      // Verify the response structure
      expect(plan).toHaveProperty('components');
      expect(Array.isArray(plan.components)).toBe(true);
      expect(plan.components[0]).toHaveProperty('name');
      expect(plan.components[0]).toHaveProperty('description');

      expect(plan).toHaveProperty('dataModel');
      expect(typeof plan.dataModel).toBe('object');

      expect(plan).toHaveProperty('implementationSteps');
      expect(Array.isArray(plan.implementationSteps)).toBe(true);
      expect(plan.implementationSteps.length).toBeGreaterThan(0);

      expect(plan).toHaveProperty('recommendedLibraries');
      expect(Array.isArray(plan.recommendedLibraries)).toBe(true);
      expect(plan.recommendedLibraries[0]).toHaveProperty('name');
      expect(plan.recommendedLibraries[0]).toHaveProperty('purpose');
    },
    { timeout: 60000 }
  );

  it(
    'should handle invalid project requirements',
    async () => {
      const invalidAnswers = {
        projectType: '', // Empty project type
        features: [], // No features
        pages: [], // No pages
        description: '', // No description
      };

      const plan = await generateProjectPlan(invalidAnswers);

      // Even with empty inputs, we should still get a valid structure
      expect(plan).toHaveProperty('components');
      expect(plan).toHaveProperty('dataModel');
      expect(plan).toHaveProperty('implementationSteps');
      expect(plan).toHaveProperty('recommendedLibraries');
    },
    { timeout: 60000 }
  );
});
