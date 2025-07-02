import OpenAI from "openai";

/**
 * Common interface for LLM responses
 */
export interface LLMResponse {
  content: string;
  usage?: {
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
  };
  model: string;
  provider: string;
}

/**
 * Common interface for LLM requests
 */
export interface LLMRequest {
  messages: Array<{
    role: "system" | "user" | "assistant";
    content: string;
  }>;
  model?: string;
  temperature?: number;
  max_tokens?: number;
  stream?: boolean;
}

/**
 * Base interface for LLM providers
 */
export interface LLMProvider {
  name: string;
  models: string[];
  complete(request: LLMRequest): Promise<LLMResponse>;
  stream(request: LLMRequest): AsyncIterable<string>;
  isConfigured(): boolean;
}

/**
 * OpenAI provider implementation
 */
export class OpenAIProvider implements LLMProvider {
  name = "openai";
  models = ["gpt-4", "gpt-4-turbo", "gpt-3.5-turbo"];
  private client?: OpenAI;

  constructor(apiKey?: string) {
    if (apiKey) {
      this.client = new OpenAI({ apiKey });
    }
  }

  isConfigured(): boolean {
    return !!this.client;
  }

  async complete(request: LLMRequest): Promise<LLMResponse> {
    if (!this.client) {
      throw new Error("OpenAI client not configured");
    }

    const response = await this.client.chat.completions.create({
      model: request.model || "gpt-4",
      messages: request.messages,
      temperature: request.temperature || 0.7,
      max_tokens: request.max_tokens,
    });

    const choice = response.choices[0];
    if (!choice?.message?.content) {
      throw new Error("No response content from OpenAI");
    }

    return {
      content: choice.message.content,
      usage: response.usage
        ? {
            prompt_tokens: response.usage.prompt_tokens,
            completion_tokens: response.usage.completion_tokens,
            total_tokens: response.usage.total_tokens,
          }
        : undefined,
      model: response.model,
      provider: this.name,
    };
  }

  async *stream(request: LLMRequest): AsyncIterable<string> {
    if (!this.client) {
      throw new Error("OpenAI client not configured");
    }

    const stream = await this.client.chat.completions.create({
      model: request.model || "gpt-4",
      messages: request.messages,
      temperature: request.temperature || 0.7,
      max_tokens: request.max_tokens,
      stream: true,
    });

    for await (const chunk of stream) {
      const content = chunk.choices[0]?.delta?.content;
      if (content) {
        yield content;
      }
    }
  }
}

/**
 * LLM service that manages multiple providers
 */
export class LLMService {
  private providers: Map<string, LLMProvider> = new Map();
  private defaultProvider?: string;

  addProvider(provider: LLMProvider): void {
    this.providers.set(provider.name, provider);
    if (!this.defaultProvider && provider.isConfigured()) {
      this.defaultProvider = provider.name;
    }
  }

  getProvider(name?: string): LLMProvider | undefined {
    const providerName = name || this.defaultProvider;
    return providerName ? this.providers.get(providerName) : undefined;
  }

  async complete(
    request: LLMRequest,
    providerName?: string,
  ): Promise<LLMResponse> {
    const provider = this.getProvider(providerName);
    if (!provider) {
      throw new Error(
        `No ${providerName ? providerName : "default"} provider available`,
      );
    }
    return provider.complete(request);
  }

  stream(request: LLMRequest, providerName?: string): AsyncIterable<string> {
    const provider = this.getProvider(providerName);
    if (!provider) {
      throw new Error(
        `No ${providerName ? providerName : "default"} provider available`,
      );
    }
    return provider.stream(request);
  }

  getAvailableProviders(): Array<{
    name: string;
    models: string[];
    configured: boolean;
  }> {
    return Array.from(this.providers.values()).map((provider) => ({
      name: provider.name,
      models: provider.models,
      configured: provider.isConfigured(),
    }));
  }
}
