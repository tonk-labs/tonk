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
import { readDoc } from "@tonk/keepsync";
import { IOHandler, IOResult } from "./types";
import { keepsyncLogger } from "../utils/keepsyncLogger";

/**
 * Keepsync handler implementation
 * Handles reading documents from the keepsync sync engine
 */
export class KeepsyncHandler<T> implements IOHandler<void, void, T> {
  /**
   * Read document contents from the specified keepsync path
   * @param path Document path in keepsync
   */
  async read(path: string): Promise<IOResult<T>> {
    try {
      const data = await readDoc<T>(path);

      if (data === undefined) {
        const error = "Document not found at the specified path";
        // Log failed read operation
        await keepsyncLogger.logRead("keepsync", path, { error });
        return {
          success: false,
          error: "Document not found at the specified path",
        };
      }

      // Log successful read operation
      await keepsyncLogger.logRead("keepsync", path, data);

      return {
        success: true,
        data,
      };
    } catch (error) {
      const errorMessage =
        error instanceof Error
          ? error.message
          : "Failed to read document from keepsync";
      // Log failed read operation
      await keepsyncLogger.logRead("keepsync", path, { error: errorMessage });
      return {
        success: false,
        error:
          error instanceof Error
            ? error.message
            : "Failed to read document from keepsync",
      };
    }
  }
}

/**
 * Factory function to create a new keepsync handler
 */
export function createKeepsyncHandler<T>(): KeepsyncHandler<T> {
  return new KeepsyncHandler<T>();
}
