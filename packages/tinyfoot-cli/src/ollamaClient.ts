export interface OllamaResponse {
  content: string;
  isCompleted?: boolean;
  [key: string]: any;
}

interface OllamaApiResponse {
  message: {
    content: string;
  };
}

export default class OllamaClient {
  private baseUrl: string;
  private model: string;

  constructor(baseUrl = 'http://localhost:11434', model = 'deepseek-r1:32b') {
    this.baseUrl = baseUrl;
    this.model = model;
  }

  setModel(model: string) {
    this.model = model;
  }

  getModel(): string {
    return this.model;
  }

  async chat(prompt: string, system?: string): Promise<OllamaResponse> {
    try {
      const response = await fetch(`${this.baseUrl}/api/chat`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          model: this.model,
          messages: [
            ...(system ? [{ role: 'system', content: system }] : []),
            { role: 'user', content: prompt }
          ],
          stream: false
        }),
      });

      if (!response.ok) {
        throw new Error(`Ollama API error: ${response.status} ${response.statusText}`);
      }

      const data = await response.json() as OllamaApiResponse;
      return {
        content: data.message.content,
        isCompleted: true
      };
    } catch (error) {
      console.error('Error in Ollama API call:', error);
      throw error;
    }
  }

  async chatStream(
    messages: Array<{ role: string; content: string }>,
    onChunk: (chunk: string) => void,
    onComplete: (fullResponse: string) => void
  ): Promise<void> {
    try {
      const response = await fetch(`${this.baseUrl}/api/chat`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          model: this.model,
          messages,
          stream: true
        }),
      });

      if (!response.ok) {
        throw new Error(`Ollama API error: ${response.status} ${response.statusText}`);
      }

      if (!response.body) {
        throw new Error('Response body is null');
      }

      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let fullResponse = '';
      
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        
        const chunk = decoder.decode(value, { stream: true });
        
        // Ollama stream returns multiple JSON objects
        const lines = chunk.split('\n').filter(line => line.trim() !== '');
        
        for (const line of lines) {
          try {
            const data = JSON.parse(line) as { message?: { content: string } };
            if (data.message && data.message.content) {
              onChunk(data.message.content);
              fullResponse += data.message.content;
            }
          } catch (e) {
            console.error('Error parsing streaming response:', e);
          }
        }
      }
      
      onComplete(fullResponse);
    } catch (error) {
      console.error('Error in Ollama streaming API call:', error);
      throw error;
    }
  }
} 