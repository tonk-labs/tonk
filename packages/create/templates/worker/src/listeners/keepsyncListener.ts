/**
 * ⚠️  CORE TEMPLATE FILE - DO NOT DELETE! ⚠️
 *
 * This file is part of the Tonk worker template infrastructure.
 * It provides file system watching capabilities for workers that need to
 * monitor file changes and automatically process them into keepsync documents.
 *
 * USAGE: Import and use this when you need to:
 * - Watch local files for changes
 * - Automatically process file content when files are added/modified
 * - Transform file data and store it in keepsync
 *
 * Even if your current worker doesn't use file watching, other workers might!
 * Keep this file for the template integrity.
 */
import { listenToDoc } from "@tonk/keepsync";
import { KeepsyncHandler } from "../io/keepsyncHandler";
import { IOResultMapper, DataMapper } from "../io/types";

/**
 * Configuration for the keepsync listener
 */
export interface KeepsyncListenerConfig<S, T, U> {
  /** Path to the keepsync document to listen to */
  inputPath: string;

  /** Path to the keepsync document to write transformed data to */
  outputPath: string;

  /** Transformation function to apply to the input data */
  transformer: (inputData: T) => U;

  /** Data mapper function for writing to the output document */
  mapper: DataMapper<S, U>;
}

/**
 * Keepsync document listener
 * Listens for changes to a keepsync document, applies transformations,
 * and writes the result to another keepsync document using IOResultMapper
 */
export class KeepsyncListener<S = any, T = any, U = any> {
  private keepsyncHandler: KeepsyncHandler<T>;
  private resultMapper: IOResultMapper<S, U>;
  private unsubscribe?: () => void;

  constructor() {
    this.resultMapper = new IOResultMapper<S, U>();
  }

  /**
   * TODO:
   * Ok, so the big issue facing us right now is that we don't want to transform the entire document every time. We need to know which piece of data changed. We need to know what's new.
   */

  /**
   * Start listening to the specified keepsync document
   * @param config Configuration for the listener
   */
  async startListening(config: KeepsyncListenerConfig<S, T, U>): Promise<void> {
    const { inputPath, outputPath, transformer, mapper } = config;

    // Set up the listener - it appears listenToDoc might return a Promise
    this.unsubscribe = await listenToDoc<T>(
      inputPath,
      (payload: { doc: T; patches: any[]; patchInfo: any; handle: any }) => {
        const { doc, patches, patchInfo, handle } = payload;
        if (doc !== undefined) {
          console.log(`KeepsyncListener: Document changed at ${inputPath}`);

          // Process the document change asynchronously
          this.processDocumentChange(
            inputPath,
            outputPath,
            transformer,
            mapper
          ).catch((error) => {
            console.error(
              "KeepsyncListener: Error processing document change:",
              error
            );
          });
        } else {
          console.log(
            `KeepsyncListener: Document at ${inputPath} was deleted or not found`
          );
        }
      }
    );

    console.log(`KeepsyncListener: Started listening to ${inputPath}`);
  }

  /**
   * Process document change asynchronously
   * @private
   */
  private async processDocumentChange(
    inputPath: string,
    outputPath: string,
    transformer: (inputData: T) => U,
    mapper: DataMapper<S, U>
  ): Promise<void> {
    try {
      // Read the document using our keepsync handler
      const result = await this.keepsyncHandler.read(inputPath);

      if (result.success && result.data) {
        // Apply the transformation
        const transformedData = transformer(result.data);

        // Create an IOResult with the transformed data
        const transformedResult = {
          success: true as const,
          data: transformedData,
        };

        // Write the transformed data using mapAndWrite
        const writeResult = await this.resultMapper.mapAndWrite(
          outputPath,
          transformedResult,
          mapper
        );

        if (writeResult.success) {
          console.log(
            `KeepsyncListener: Successfully wrote transformed data to ${outputPath}`
          );
        } else {
          console.error(
            `KeepsyncListener: Failed to write to ${outputPath}: ${writeResult.error}`
          );
        }
      } else {
        console.error(
          `KeepsyncListener: Failed to read from ${inputPath}: ${result.error}`
        );
      }
    } catch (error) {
      console.error("KeepsyncListener: Error in processDocumentChange:", error);
      throw error;
    }
  }

  /**
   * Stop listening to the keepsync document
   */
  stopListening(): void {
    if (this.unsubscribe) {
      this.unsubscribe();
      this.unsubscribe = undefined;
      console.log("KeepsyncListener: Stopped listening");
    }
  }

  /**
   * Check if the listener is currently active
   */
  isListening(): boolean {
    return this.unsubscribe !== undefined;
  }
}

/**
 * Factory function to create a new keepsync listener
 */
export function createKeepsyncListener<
  S = any,
  T = any,
  U = any
>(): KeepsyncListener<S, T, U> {
  return new KeepsyncListener<S, T, U>();
}

/**
 * Utility function to create a simple keepsync listener with minimal configuration
 * @param inputPath Path to listen to
 * @param outputPath Path to write to
 * @param transformer Transformation function
 * @param mapper Data mapper function
 * @returns Promise that resolves to the created and started listener
 */
export async function createAndStartListener<S = any, T = any, U = any>(
  inputPath: string,
  outputPath: string,
  transformer: (inputData: T) => U,
  mapper: DataMapper<S, U>
): Promise<KeepsyncListener<S, T, U>> {
  const listener = createKeepsyncListener<S, T, U>();

  await listener.startListening({
    inputPath,
    outputPath,
    transformer,
    mapper,
  });

  return listener;
}
