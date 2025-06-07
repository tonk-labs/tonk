import chokidar from "chokidar";
import { readFile } from "fs/promises";
import { IOResultMapper, DataMapper } from "../io/types";

/**
 * Configuration for the file listener
 */
export interface FileListenerConfig<S, T, U> {
  /** Absolute path to the file or folder to listen to */
  listeningPath: string;

  /** Function that transforms a file path into a keepsync document path */
  pathTransformer: (filePath: string) => string;

  /** Transformation function to apply to the file content */
  transformer: (fileContent: string, filePath: string) => U;

  /** Data mapper function for writing to the output document */
  mapper: DataMapper<S, U>;

  /** Optional chokidar options */
  watchOptions?: any;
}

/**
 * File system listener
 * Listens for changes to files or folders using chokidar, applies transformations,
 * and writes the result to keepsync documents using IOResultMapper
 */
export class FileListener<S = any, T = any, U = any> {
  private resultMapper: IOResultMapper<S, U>;
  private watcher?: any;

  constructor() {
    this.resultMapper = new IOResultMapper<S, U>();
  }

  /**
   * Start listening to the specified file or folder path
   * @param config Configuration for the listener
   */
  async startListening(config: FileListenerConfig<S, T, U>): Promise<void> {
    const {
      listeningPath,
      pathTransformer,
      transformer,
      mapper,
      watchOptions = {},
    } = config;

    // Default chokidar options
    const defaultOptions: any = {
      ignored: /(^|[\/\\])\../, // ignore dotfiles
      persistent: true,
      ignoreInitial: false,
      ...watchOptions,
    };

    // Create the chokidar watcher
    this.watcher = chokidar.watch(listeningPath, defaultOptions);

    // Set up event listeners
    this.watcher
      .on("add", (filePath: string) => {
        console.log(`FileListener: File added: ${filePath}`);
        this.processFileChange(
          "add",
          filePath,
          pathTransformer,
          transformer,
          mapper
        ).catch((error) => {
          console.error("FileListener: Error processing file add:", error);
        });
      })
      .on("change", (filePath: string) => {
        console.log(`FileListener: File changed: ${filePath}`);
        this.processFileChange(
          "change",
          filePath,
          pathTransformer,
          transformer,
          mapper
        ).catch((error) => {
          console.error("FileListener: Error processing file change:", error);
        });
      })
      .on("unlink", (filePath: string) => {
        console.log(`FileListener: File removed: ${filePath}`);
        this.processFileChange(
          "unlink",
          filePath,
          pathTransformer,
          transformer,
          mapper
        ).catch((error) => {
          console.error("FileListener: Error processing file removal:", error);
        });
      })
      .on("addDir", (dirPath: string) => {
        console.log(`FileListener: Directory added: ${dirPath}`);
        // Optionally handle directory additions
      })
      .on("unlinkDir", (dirPath: string) => {
        console.log(`FileListener: Directory removed: ${dirPath}`);
        // Optionally handle directory removals
      })
      .on("error", (error: Error) => {
        console.error("FileListener: Watcher error:", error);
      })
      .on("ready", () => {
        console.log(`FileListener: Started listening to ${listeningPath}`);
      });
  }

  /**
   * Process file change asynchronously
   * @private
   */
  private async processFileChange(
    eventType: "add" | "change" | "unlink",
    filePath: string,
    pathTransformer: (filePath: string) => string,
    transformer: (fileContent: string, filePath: string) => U,
    mapper: DataMapper<S, U>
  ): Promise<void> {
    try {
      // Transform the file path to get the document path
      const documentPath = pathTransformer(filePath);

      if (eventType === "unlink") {
        // Handle file deletion - you might want to delete the corresponding document
        // or write some special marker indicating the file was deleted
        console.log(
          `FileListener: File ${filePath} was deleted, document path: ${documentPath}`
        );
        // For now, we'll just log this. You might want to implement deletion logic here.
        return;
      }

      // Read the file content
      const fileContent = await readFile(filePath, "utf-8");

      // Apply the transformation
      const transformedData = transformer(fileContent, filePath);

      // Create an IOResult with the transformed data
      const transformedResult = {
        success: true as const,
        data: transformedData,
      };

      // Write the transformed data using mapAndWrite
      const writeResult = await this.resultMapper.mapAndWrite(
        documentPath,
        transformedResult,
        mapper
      );

      if (writeResult.success) {
        console.log(
          `FileListener: Successfully wrote transformed data to ${documentPath}`
        );
      } else {
        console.error(
          `FileListener: Failed to write to ${documentPath}: ${writeResult.error}`
        );
      }
    } catch (error) {
      console.error("FileListener: Error in processFileChange:", error);
      throw error;
    }
  }

  /**
   * Stop listening to file changes
   */
  async stopListening(): Promise<void> {
    if (this.watcher) {
      await this.watcher.close();
      this.watcher = undefined;
      console.log("FileListener: Stopped listening");
    }
  }

  /**
   * Check if the listener is currently active
   */
  isListening(): boolean {
    return this.watcher !== undefined;
  }
}

/**
 * Factory function to create a new file listener
 */
export function createFileListener<S = any, T = any, U = any>(): FileListener<
  S,
  T,
  U
> {
  return new FileListener<S, T, U>();
}

/**
 * Utility function to create a simple file listener with minimal configuration
 * @param listeningPath Absolute path to listen to
 * @param pathTransformer Function to transform file paths to document paths
 * @param transformer Transformation function for file content
 * @param mapper Data mapper function
 * @param watchOptions Optional chokidar options
 * @returns Promise that resolves to the created and started listener
 */
export async function createAndStartFileListener<S = any, T = any, U = any>(
  listeningPath: string,
  pathTransformer: (filePath: string) => string,
  transformer: (fileContent: string, filePath: string) => U,
  mapper: DataMapper<S, U>,
  watchOptions?: any
): Promise<FileListener<S, T, U>> {
  const listener = createFileListener<S, T, U>();

  await listener.startListening({
    listeningPath,
    pathTransformer,
    transformer,
    mapper,
    watchOptions,
  });

  return listener;
}
