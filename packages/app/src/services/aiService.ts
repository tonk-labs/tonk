import { createGroq } from '@ai-sdk/groq';
import { streamText } from 'ai';

export interface StreamingResponse {
  textStream: AsyncIterable<string>;
  fullTextPromise: Promise<string>;
}

export class AIService {
  private groqClient = createGroq({
    apiKey: import.meta.env.VITE_GROQ_API_KEY || '',
  });

  private model = this.groqClient('moonshotai/kimi-k2-instruct');

  async streamChat(
    messages: Array<{ role: 'user' | 'assistant'; content: string }>
  ): Promise<StreamingResponse> {
    const result = streamText({
      model: this.model,
      messages,
      temperature: 0.7,
    });

    return {
      textStream: result.textStream,
      fullTextPromise: result.text,
    };
  }

  async generateResponse(prompt: string): Promise<StreamingResponse> {
    return this.streamChat([{ role: 'user', content: prompt }]);
  }
}

export const aiService = new AIService();
