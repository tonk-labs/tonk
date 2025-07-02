import { readDoc, listenToDoc, ls } from "@tonk/keepsync";
import { vectorService } from "./vectorService";
import { csvQueryService } from "./csvQueryService";

interface SourceDocument {
  id?: string;
  title?: string;
  content?: string;
  rawCsvContent?: string;
  metadata?: {
    type?: "text" | "pdf" | "web" | "csv";
    [key: string]: any;
  };
  [key: string]: any;
}

export class IndexingService {
  private listeners: Map<string, () => void> = new Map();
  private isInitialized = false;
  private watchedPaths = new Set<string>();
  private pendingIndexing = new Set<string>();
  private indexedVectorSources = new Set<string>();
  private indexedCsvSources = new Set<string>();
  private lastIndexingActivity = new Date();
  private totalSourcesFound = 0;

  // Batch progress tracking
  private totalBatches = 0;
  private processedBatches = 0;
  private currentBatchProgress:
    | {
        sourceId: string;
        sourceTitle: string;
        batchIndex: number;
        totalChunks: number;
        processedChunks: number;
      }
    | undefined;
  private batchCalculationComplete = false;
  
  // Content hash tracking for change detection
  private sourceContentHashes = new Map<string, string>();

  async initialize(): Promise<void> {
    if (this.isInitialized) return;

    console.log("Indexing Service: Initializing...");

    // Initialize vector service
    await vectorService.initialize();

    // Initialize tracked sources by scanning existing CSV sources
    // (Vector sources will be added as they're discovered and indexed)
    await this.initializeTrackedSources();

    this.isInitialized = true;
    console.log("Indexing Service: Initialized");
  }

  /**
   * Initialize tracked sources from existing data
   */
  private async initializeTrackedSources(): Promise<void> {
    try {
      // Get existing vector sources from ChromaDB
      const existingVectorSources = await vectorService.getExistingSources();
      existingVectorSources.forEach((sourceId) => {
        this.indexedVectorSources.add(sourceId);
      });

      // Get existing CSV sources and add them to our tracking
      const existingCsvSources = csvQueryService.getAllSources();
      existingCsvSources.forEach((sourceId) => {
        this.indexedCsvSources.add(sourceId);
      });

      // Initialize content hashes for existing sources by reading current documents
      await this.initializeContentHashes();

      console.log(
        `Indexing Service: Initialized tracking for ${existingVectorSources.size} vector sources and ${existingCsvSources.length} CSV sources`,
      );
    } catch (error) {
      console.warn("Failed to initialize tracked sources:", error);
    }
  }

  /**
   * Initialize content hashes for existing sources
   */
  private async initializeContentHashes(): Promise<void> {
    try {
      const dataPath = "tonkbook/data";
      const docNode = await ls(dataPath);

      if (docNode && docNode.children) {
        const docChildren = docNode.children.filter((child) => child.type === "doc");
        
        for (const child of docChildren) {
          const fullPath = `${dataPath}/${child.name}`;
          try {
            const doc = await readDoc<SourceDocument>(fullPath);
            if (doc && (doc.content || doc.rawCsvContent)) {
              const sourceId = doc.id || fullPath.replace(/\//g, "_");
              const content = doc.content || doc.rawCsvContent || "";
              
              // Calculate and store initial hash (this will also handle the "first time" case)
              this.hasContentChanged(sourceId, content);
            }
          } catch (error) {
            console.warn(`Failed to read document ${fullPath} for hash initialization:`, error);
          }
        }
      }
    } catch (error) {
      console.warn("Failed to initialize content hashes:", error);
    }
  }

  /**
   * Start listening for changes in a specific source document path
   */
  async watchSourceDocument(documentPath: string): Promise<void> {
    if (this.listeners.has(documentPath)) {
      console.log(`Already watching ${documentPath}`);
      return;
    }

    try {
      const unsubscribe = await listenToDoc(
        documentPath,
        async (payload: any) => {
          const doc = payload.doc;
          if (doc && (doc.content || doc.rawCsvContent)) {
            const sourceId = doc.id || documentPath.replace(/\//g, "_");
            const content = doc.content || doc.rawCsvContent || "";
            
            // Only re-index if content has actually changed
            if (this.hasContentChanged(sourceId, content)) {
              console.log(`Source document content changed: ${documentPath}`);
              this.pendingIndexing.add(documentPath);
              this.lastIndexingActivity = new Date();
              await this.indexSource(doc, documentPath);
            } else {
              console.log(`Source document updated but content unchanged: ${documentPath}`);
            }
          }
        },
      );

      this.listeners.set(documentPath, unsubscribe);
      console.log(`Indexing Service: Started watching ${documentPath}`);

      // Also index the current document if it exists
      const currentDoc = await readDoc<SourceDocument>(documentPath);
      if (currentDoc && (currentDoc.content || currentDoc.rawCsvContent)) {
        const sourceId = currentDoc.id || documentPath.replace(/\//g, "_");
        const sourceType = currentDoc.metadata?.type || "text";

        // Check if already tracked to avoid re-indexing during startup
        const isAlreadyTracked =
          (sourceType === "csv" && this.indexedCsvSources.has(sourceId)) ||
          (sourceType !== "csv" && this.indexedVectorSources.has(sourceId));

        // Check if content has changed (even for tracked sources)
        const content = currentDoc.content || currentDoc.rawCsvContent || "";
        const contentHasChanged = this.hasContentChanged(sourceId, content);

        if (!isAlreadyTracked || contentHasChanged) {
          if (isAlreadyTracked && contentHasChanged) {
            console.log(`Content changed for ${documentPath}, re-indexing...`);
          } else {
            console.log(`Indexing new document: ${documentPath}`);
          }
          this.pendingIndexing.add(documentPath);
          this.lastIndexingActivity = new Date();
          await this.indexSource(currentDoc, documentPath);
        } else {
          console.log(
            `Document already indexed and unchanged: ${documentPath} (${sourceId})`,
          );
        }
      }
    } catch (error) {
      console.error(`Failed to watch source document ${documentPath}:`, error);
    }
  }

  /**
   * Stop watching a specific source document
   */
  stopWatching(documentPath: string): void {
    const unsubscribe = this.listeners.get(documentPath);
    if (unsubscribe) {
      unsubscribe();
      this.listeners.delete(documentPath);
      console.log(`Indexing Service: Stopped watching ${documentPath}`);
    }
  }

  /**
   * Calculate a simple hash of content for change detection
   */
  private calculateContentHash(content: string): string {
    let hash = 0;
    for (let i = 0; i < content.length; i++) {
      const char = content.charCodeAt(i);
      hash = ((hash << 5) - hash) + char;
      hash = hash & hash; // Convert to 32-bit integer
    }
    return hash.toString();
  }

  /**
   * Check if source content has changed since last indexing
   */
  private hasContentChanged(sourceId: string, content: string): boolean {
    const currentHash = this.calculateContentHash(content);
    const previousHash = this.sourceContentHashes.get(sourceId);
    
    if (!previousHash) {
      // No previous hash, content is "new"
      this.sourceContentHashes.set(sourceId, currentHash);
      return true;
    }
    
    if (currentHash !== previousHash) {
      // Content has changed, update hash
      this.sourceContentHashes.set(sourceId, currentHash);
      return true;
    }
    
    // Content unchanged
    return false;
  }

  /**
   * Pre-calculate total batches across all discovered sources
   */
  private async calculateTotalBatches(): Promise<void> {
    if (this.batchCalculationComplete) return;

    console.log("Indexing Service: Pre-calculating total batches...");
    let totalChunks = 0;
    const batchSize = 25; // Same as vector service batch size

    try {
      const dataPath = "tonkbook/data";
      const docNode = await ls(dataPath);

      if (docNode && docNode.children) {
        const docChildren = docNode.children.filter(
          (child) => child.type === "doc",
        );

        for (const child of docChildren) {
          const fullPath = `${dataPath}/${child.name}`;
          try {
            const doc = await readDoc<SourceDocument>(fullPath);
            if (doc && (doc.content || doc.rawCsvContent)) {
              const sourceType = doc.metadata?.type || "text";

              if (
                sourceType === "text" ||
                sourceType === "pdf" ||
                sourceType === "web"
              ) {
                // Calculate chunks for vector sources
                const chunkCount = vectorService.calculateChunkCount(
                  doc.content!,
                );
                totalChunks += chunkCount;
                console.log(`Calculated ${chunkCount} chunks for ${fullPath}`);
              } else if (sourceType === "csv") {
                // CSV sources are treated as 1 "batch" each
                totalChunks += 1;
                console.log(`CSV source ${fullPath} counts as 1 batch`);
              }
            }
          } catch (error) {
            console.warn(
              `Failed to read document ${fullPath} for batch calculation:`,
              error,
            );
          }
        }
      }

      this.totalBatches = Math.ceil(totalChunks / batchSize);
      this.batchCalculationComplete = true;

      console.log(
        `Indexing Service: Pre-calculated ${totalChunks} total chunks = ${this.totalBatches} batches`,
      );
    } catch (error) {
      console.error("Failed to calculate total batches:", error);
      // Fallback to 0, will calculate dynamically if needed
      this.totalBatches = 0;
    }
  }

  /**
   * Index a source document into the appropriate storage system
   */
  private async indexSource(
    doc: SourceDocument,
    documentPath: string,
  ): Promise<void> {
    try {
      const sourceId = doc.id || documentPath.replace(/\//g, "_");
      const sourceType = doc.metadata?.type || "text";
      const title = doc.title || documentPath.split("/").pop() || "Unknown";

      console.log(
        `Indexing source ${sourceId} of type ${sourceType} from ${documentPath}`,
      );

      // Create a standardized source object
      const source = {
        id: sourceId,
        title: title,
        metadata: { type: sourceType as any, ...doc.metadata },
      };

      // Track the starting batch count for this source
      const startingBatchCount = this.processedBatches;

      // Create batch progress callback using pre-calculated totals
      const onBatchProgress = (
        batchIndex: number,
        processedChunks: number,
        totalChunks: number,
      ) => {
        this.currentBatchProgress = {
          sourceId,
          sourceTitle: title,
          batchIndex,
          totalChunks,
          processedChunks,
        };

        // Update global processed batch count based on completed batches within this source
        this.processedBatches = startingBatchCount + batchIndex;

        this.lastIndexingActivity = new Date();
      };

      switch (sourceType) {
        case "text":
        case "pdf":
        case "web":
          await vectorService.addDocument(
            source,
            doc.content!,
            onBatchProgress,
          );
          this.indexedVectorSources.add(sourceId);
          // Ensure the processed batches count reflects all batches for this source
          const finalChunkCount = vectorService.calculateChunkCount(
            doc.content!,
          );
          const finalBatchCount = Math.ceil(finalChunkCount / 25);
          this.processedBatches = startingBatchCount + finalBatchCount;
          break;
        case "csv":
          // For CSV sources, use rawCsvContent instead of content
          const csvContent = doc.rawCsvContent || doc.content;
          if (csvContent) {
            await csvQueryService.addCSVSource(source, csvContent);
            this.indexedCsvSources.add(sourceId);
            // CSV sources count as 1 batch, so increment the global counter
            this.processedBatches += 1;
            this.currentBatchProgress = {
              sourceId,
              sourceTitle: title,
              batchIndex: 1,
              totalChunks: 1,
              processedChunks: 1,
            };
          } else {
            console.warn(
              `No rawCsvContent or content found for CSV source ${sourceId}`,
            );
          }
          break;
        default:
          // Default to text indexing for unknown types
          await vectorService.addDocument(
            { ...source, metadata: { type: "text" as any } },
            doc.content!,
            onBatchProgress,
          );
          this.indexedVectorSources.add(sourceId);
          // Ensure the processed batches count reflects all batches for this source
          const defaultFinalChunkCount = vectorService.calculateChunkCount(
            doc.content!,
          );
          const defaultFinalBatchCount = Math.ceil(defaultFinalChunkCount / 25);
          this.processedBatches = startingBatchCount + defaultFinalBatchCount;
          break;
      }

      console.log(`Successfully indexed source ${sourceId}`);

      // Remove from pending
      this.pendingIndexing.delete(documentPath);
      this.lastIndexingActivity = new Date();
    } catch (error) {
      console.error(`Failed to index source from ${documentPath}:`, error);
      // Remove from pending even if failed to avoid stuck state
      this.pendingIndexing.delete(documentPath);
    }
  }

  /**
   * Watch for source documents in the tonkbook data directory
   */
  async watchSourcesDirectory(): Promise<void> {
    console.log("Indexing Service: Setting up directory monitoring...");

    // Scan and index existing documents in /tonkbook/data
    await this.scanAndIndexExistingDocs();

    // Set up polling to check for new documents periodically
    setInterval(async () => {
      await this.scanAndIndexExistingDocs();
    }, 30000); // Check every 30 seconds

    console.log("Indexing Service: Directory monitoring active");
  }

  /**
   * Scan the /tonkbook/data path for documents and index them
   */
  private async scanAndIndexExistingDocs(): Promise<void> {
    try {
      console.log("Scanning /tonkbook/data for documents...");

      // Try to list documents in the tonkbook/data path
      const dataPath = "tonkbook/data";
      try {
        const docNode = await ls(dataPath);
        console.log(`ls result for ${dataPath}:`, docNode);

        // Handle DocNode structure
        if (docNode && docNode.children) {
          const children = docNode.children;
          const docChildren = children.filter((child) => child.type === "doc");
          this.totalSourcesFound = docChildren.length;
          console.log(
            `Found ${docChildren.length} source documents in ${dataPath}`,
          );

          // Pre-calculate total batches before starting indexing
          await this.calculateTotalBatches();

          for (const child of docChildren) {
            const fullPath = `${dataPath}/${child.name}`;
            if (!this.watchedPaths.has(fullPath)) {
              console.log(`Setting up watch for document: ${fullPath}`);
              this.pendingIndexing.add(fullPath);
              this.lastIndexingActivity = new Date();
              await this.watchSourceDocument(fullPath);
              this.watchedPaths.add(fullPath);
            }
          }
        } else {
          console.log("No children found in DocNode or invalid structure");
        }
      } catch (error) {
        console.log(
          "No documents found in /tonkbook/data or path doesn't exist yet:",
          error.message,
        );
      }
    } catch (error) {
      console.error("Error scanning for documents:", error);
    }
  }

  /**
   * Get indexing statistics with batch-level progress
   */
  async getStats(): Promise<{
    vectorSources: { count: number };
    csvSources: { count: number; sources: string[] };
    progress: {
      pendingCount: number;
      indexedCount: number;
      totalDiscovered: number;
      isIndexing: boolean;
      lastActivity: string;
      batches: {
        totalBatches: number;
        processedBatches: number;
        pendingBatches: number;
        currentBatchProgress?: {
          sourceId: string;
          sourceTitle: string;
          batchIndex: number;
          totalChunks: number;
          processedChunks: number;
        };
      };
    };
  }> {
    // Get CSV sources from the service to stay consistent
    const csvSources = csvQueryService.getAllSources();

    // Use our tracked counts for actual source documents (not chunks)
    const vectorSourceCount = this.indexedVectorSources.size;
    const csvSourceCount = this.indexedCsvSources.size;
    const totalIndexed = vectorSourceCount + csvSourceCount;
    const totalDiscovered = this.totalSourcesFound; // Total sources found in directory
    const pendingCount = totalDiscovered - totalIndexed; // Remaining to index

    // Consider indexing active if there's pending work or recent activity (last 10 seconds)
    const isIndexing =
      pendingCount > 0 ||
      Date.now() - this.lastIndexingActivity.getTime() < 10000;

    // Calculate batch statistics
    const pendingBatches = Math.max(
      0,
      this.totalBatches - this.processedBatches,
    );

    return {
      vectorSources: { count: vectorSourceCount },
      csvSources: {
        count: csvSourceCount,
        sources: csvSources,
      },
      progress: {
        pendingCount,
        indexedCount: totalIndexed,
        totalDiscovered,
        isIndexing,
        lastActivity: this.lastIndexingActivity.toISOString(),
        batches: {
          totalBatches: this.totalBatches,
          processedBatches: this.processedBatches,
          pendingBatches,
          currentBatchProgress: this.currentBatchProgress,
        },
      },
    };
  }

  /**
   * Clean up all listeners
   */
  cleanup(): void {
    console.log("Indexing Service: Cleaning up listeners...");
    for (const [path, unsubscribe] of this.listeners) {
      unsubscribe();
      console.log(`Stopped watching ${path}`);
    }
    this.listeners.clear();
  }
}

export const indexingService = new IndexingService();
