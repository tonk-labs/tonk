import type {ParsedFunction} from './parser.js';

interface OllamaEmbeddingResponse {
  embedding: number[];
}

export async function getEmbedding(text: string): Promise<number[]> {
  try {
    const response = await fetch('http://localhost:11434/api/embeddings', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        model: 'deepseek-r1:8b',
        prompt: text,
      }),
    });

    if (!response.ok) {
      throw new Error(`Ollama request failed: ${response.statusText}`);
    }

    const data = (await response.json()) as OllamaEmbeddingResponse;
    return data.embedding;
  } catch (error) {
    console.error('Error getting embedding:', error);
    throw error;
  }
}

// Calculate cosine similarity between two vectors
export function cosineSimilarity(vec1: number[], vec2: number[]): number {
  if (vec1.length !== vec2.length) {
    throw new Error('Vectors must have the same length');
  }

  let dotProduct = 0;
  let norm1 = 0;
  let norm2 = 0;

  for (let i = 0; i < vec1.length; i++) {
    dotProduct += vec1[i] * vec2[i];
    norm1 += vec1[i] * vec1[i];
    norm2 += vec2[i] * vec2[i];
  }

  // Avoid division by zero
  if (norm1 === 0 || norm2 === 0) return 0;

  return dotProduct / (Math.sqrt(norm1) * Math.sqrt(norm2));
}
