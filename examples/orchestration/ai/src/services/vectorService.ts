import { ChromaClient } from "chromadb";

interface SourceDocument {
  id: string;
  title: string;
  metadata: {
    type: "text" | "pdf" | "web" | "csv";
    [key: string]: any;
  };
}

interface DocumentChunk {
  id: string;
  content: string;
  metadata: {
    sourceId: string;
    sourceType: SourceDocument["metadata"]["type"];
    chunkIndex: number;
    title: string;
  };
}

interface SearchResult {
  content: string;
  metadata: DocumentChunk["metadata"];
  score: number;
}

export class VectorService {
  private client: ChromaClient;
  private collection: any;
  private initialized = false;

  constructor() {
    this.client = new ChromaClient({
      port: 8888,
    });
  }

  async initialize(): Promise<void> {
    if (this.initialized) return;

    try {
      console.log(
        "Vector Service: Initializing connection to Chroma at http://localhost:8888",
      );
      this.collection = await this.client.getOrCreateCollection({
        name: "tonkbook_sources",
        metadata: { description: "Tonkbook source documents for RAG" },
      });
      console.log("Vector Service: Successfully initialized collection");
      this.initialized = true;
    } catch (error) {
      console.error("Failed to initialize vector service:", error);
      throw error;
    }
  }

  /**
   * Calculate the number of chunks a document would be split into without actually processing it
   */
  calculateChunkCount(content: string): number {
    const maxChunkSize = 800; // characters
    const overlap = 150; // character overlap between chunks

    let startIndex = 0;
    let chunkCount = 0;

    while (startIndex < content.length) {
      const endIndex = Math.min(startIndex + maxChunkSize, content.length);
      const chunkContent = content.slice(startIndex, endIndex);

      // Try to break at sentence boundaries (same logic as chunkDocument)
      let actualEndIndex = endIndex;
      if (endIndex < content.length) {
        const lastSentenceEnd = chunkContent.lastIndexOf(". ");
        if (lastSentenceEnd > maxChunkSize * 0.7) {
          actualEndIndex = startIndex + lastSentenceEnd + 1;
        }
      }

      const finalContent = content.slice(startIndex, actualEndIndex).trim();
      if (finalContent.length > 0) {
        chunkCount++;
      }

      startIndex = Math.max(actualEndIndex - overlap, startIndex + 1);
      if (startIndex >= actualEndIndex) break;
    }

    return chunkCount;
  }

  /**
   * Get all source IDs that are currently indexed in the vector database
   */
  async getExistingSources(): Promise<Set<string>> {
    await this.initialize();

    try {
      // Get all documents and extract unique source IDs
      const results = await this.collection.get({
        include: ["metadatas"],
      });

      const sourceIds = new Set<string>();

      if (results.metadatas) {
        for (const metadata of results.metadatas) {
          if (metadata && metadata.sourceId) {
            sourceIds.add(metadata.sourceId);
          }
        }
      }

      console.log(
        `Vector Service: Found ${sourceIds.size} existing sources in ChromaDB`,
      );
      return sourceIds;
    } catch (error) {
      console.error("Failed to get existing sources from ChromaDB:", error);
      return new Set();
    }
  }

  /**
   * Add a source document to the vector database
   */
  async addDocument(
    source: SourceDocument,
    content: string,
    onBatchProgress?: (
      batchIndex: number,
      totalBatches: number,
      processedChunks: number,
      totalChunks: number,
    ) => void,
  ): Promise<void> {
    console.log(`Vector Service: Adding document for source ${source.id}`);
    await this.initialize();

    // Remove existing chunks for this source first
    await this.removeDocument(source.id);

    // Calculate total chunks
    const totalChunks = this.calculateChunkCount(content);
    console.log(
      `Vector Service: Will create ${totalChunks} chunks for source ${source.id}`,
    );

    if (totalChunks === 0) return;

    // Stream chunks and process immediately
    const batchSize = 25;
    const totalBatches = Math.ceil(totalChunks / batchSize);
    let processedChunks = 0;
    let batchIndex = 0;

    for await (const chunkBatch of this.streamChunksInBatches(
      content,
      source,
      batchSize,
    )) {
      batchIndex++;

      console.log(
        `Vector Service: Adding batch ${batchIndex}/${totalBatches} (${chunkBatch.length} chunks)`,
      );

      // Report progress before processing batch
      if (onBatchProgress) {
        onBatchProgress(
          batchIndex - 1,
          totalBatches,
          processedChunks,
          totalChunks,
        );
      }

      try {
        await this.collection.add({
          ids: chunkBatch.map((chunk) => chunk.id),
          documents: chunkBatch.map((chunk) => chunk.content),
          metadatas: chunkBatch.map((chunk) => chunk.metadata),
        });

        processedChunks += chunkBatch.length;
        console.log(`Vector Service: Successfully added batch ${batchIndex}`);

        // Report progress after processing batch
        if (onBatchProgress) {
          onBatchProgress(
            batchIndex,
            totalBatches,
            processedChunks,
            totalChunks,
          );
        }

        // Small delay between batches to allow garbage collection
        if (batchIndex < totalBatches) {
          await new Promise((resolve) => setTimeout(resolve, 100));
        }
      } catch (error) {
        console.error(
          `Vector Service: Failed to add batch ${batchIndex}:`,
          error,
        );
        throw error;
      }
    }

    console.log(
      `Vector Service: Successfully added all ${processedChunks} chunks for source ${source.id}`,
    );
  }

  /**
   * Remove a source document from the vector database
   */
  async removeDocument(sourceId: string): Promise<void> {
    await this.initialize();

    try {
      await this.collection.delete({
        where: { sourceId },
      });
      console.log(`Vector Service: Removed source ${sourceId}`);
    } catch (error) {
      // Don't throw error if document doesn't exist
      console.log(
        `Vector Service: Source ${sourceId} not found or already removed`,
      );
    }
  }

  /**
   * Search for relevant documents using vector similarity
   */
  async search(query: string, topK: number = 5): Promise<SearchResult[]> {
    await this.initialize();

    const results = await this.collection.query({
      queryTexts: [query],
      nResults: topK,
      include: ["documents", "metadatas", "distances"],
    });

    if (
      !results.documents?.[0] ||
      !results.metadatas?.[0] ||
      !results.distances?.[0]
    ) {
      return [];
    }

    return results.documents[0].map((doc: string, index: number) => ({
      content: doc,
      metadata: results.metadatas![0][index] as DocumentChunk["metadata"],
      score: 1 - (results.distances![0][index] || 0), // Convert distance to similarity score
    }));
  }

  /**
   * Stream document chunks in batches
   */
  private async *streamChunksInBatches(
    content: string,
    source: SourceDocument,
    batchSize: number = 25,
  ): AsyncGenerator<DocumentChunk[], void, unknown> {
    const maxChunkSize = 800;
    const overlap = 150;

    let startIndex = 0;
    let chunkIndex = 0;
    let currentBatch: DocumentChunk[] = [];

    while (startIndex < content.length) {
      const endIndex = Math.min(startIndex + maxChunkSize, content.length);
      const chunkContent = content.slice(startIndex, endIndex);

      // Try to break at sentence boundaries
      let actualEndIndex = endIndex;
      if (endIndex < content.length) {
        const lastSentenceEnd = chunkContent.lastIndexOf(". ");
        if (lastSentenceEnd > maxChunkSize * 0.7) {
          actualEndIndex = startIndex + lastSentenceEnd + 1;
        }
      }

      const finalContent = content.slice(startIndex, actualEndIndex).trim();

      if (finalContent.length > 0) {
        currentBatch.push({
          id: `${source.id}_chunk_${chunkIndex}`,
          content: finalContent,
          metadata: {
            sourceId: source.id,
            sourceType: source.metadata.type,
            chunkIndex,
            title: source.title,
          },
        });
        chunkIndex++;

        // Yield batch when full
        if (currentBatch.length >= batchSize) {
          yield currentBatch;
          currentBatch = []; // Clear for next batch
        }
      }

      startIndex = Math.max(actualEndIndex - overlap, startIndex + 1);
      if (startIndex >= actualEndIndex) break;
    }

    // Yield remaining chunks if any
    if (currentBatch.length > 0) {
      yield currentBatch;
    }
  }

  /**
   * Get collection statistics
   */
  async getStats(): Promise<{ count: number }> {
    await this.initialize();

    const count = await this.collection.count();
    return { count };
  }
}

export const vectorService = new VectorService();
