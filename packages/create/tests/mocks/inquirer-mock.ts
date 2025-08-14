import { vi } from 'vitest';

export const createMockInquirer = () => {
  const mockPrompt = vi.fn();

  // Default responses for different question types
  const defaultResponses = {
    platform: 'react',
    projectName: 'test-app',
    description: 'Test application',
  };

  mockPrompt.mockImplementation((questions: any[]) => {
    const responses: any = {};

    questions.forEach(question => {
      if (
        question.name &&
        defaultResponses[question.name as keyof typeof defaultResponses]
      ) {
        responses[question.name] =
          defaultResponses[question.name as keyof typeof defaultResponses];
      } else {
        // Provide sensible defaults for unknown questions
        responses[question.name] = question.default || 'mock-answer';
      }
    });

    return Promise.resolve(responses);
  });

  return {
    prompt: mockPrompt,

    // Test utilities
    setMockResponse: (questionName: string, response: any) => {
      (defaultResponses as any)[questionName] = response;
    },
    setMockResponses: (responses: Record<string, any>) => {
      Object.assign(defaultResponses, responses);
    },
    getMockResponses: () => ({ ...defaultResponses }),
  };
};
