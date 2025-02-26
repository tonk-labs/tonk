import { describe, it, expect, beforeEach, vi, beforeAll } from 'vitest';

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

  it('should generate a valid implementation plan', async () => {
    const mockAnswers = {
      projectType: 'web',
      features: ['authentication', 'real-time-sync'],
      pages: ['home', 'dashboard'],
      description: 'A collaborative note-taking app'
    };

    const plan = await generateImplementationPlan(mockAnswers);

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
  }, { timeout: 60000 });

  it('should handle invalid project requirements', async () => {
    const invalidAnswers = {
      projectType: '',  // Empty project type
      features: [],     // No features
      pages: [],        // No pages
      description: ''   // No description
    };

    const plan = await generateImplementationPlan(invalidAnswers);
    
    // Even with empty inputs, we should still get a valid structure
    expect(plan).toHaveProperty('components');
    expect(plan).toHaveProperty('dataModel');
    expect(plan).toHaveProperty('implementationSteps');
    expect(plan).toHaveProperty('recommendedLibraries');
  }, { timeout: 60000 });
});

// Helper function that was previously inline
async function generateImplementationPlan(answers: {
  projectType: string;
  features: string[];
  pages: string[];
  description: string;
}) {
  const prompt = `You are an expert in full stack development and local-first tooling. Based on the following project requirements, generate a structured implementation plan.
    Prioritize Automerge and WebSocket for local-first development (found in src/lib/sync-engine/), and Tailwind for styling.
    
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

  if (!planJson.components || !planJson.dataModel ||
    !planJson.implementationSteps || !planJson.recommendedLibraries) {
    throw new Error('Invalid response structure from LLM');
  }

  return planJson;
}
