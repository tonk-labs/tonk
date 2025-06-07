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
import { promises as fs } from "fs";
import { IOHandler, IOResult } from "./types";
import { keepsyncLogger } from "../utils/keepsyncLogger";

/**
 * Filesystem handler implementation
 * Handles local file system operations
 */
export class FSHandler implements IOHandler<string, void, string> {
  /**
   * Read file contents from the specified location
   * @param location File path relative to base path or absolute path
   */
  async read(filepath: string): Promise<IOResult<string>> {
    try {
      const data = await fs.readFile(filepath, "utf-8");

      // Log successful read operation
      await keepsyncLogger.logRead("fs", filepath, data);

      return {
        success: true,
        data,
      };
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : "Failed to read file";
      // Log failed read operation
      await keepsyncLogger.logRead("fs", filepath, { error: errorMessage });
      return {
        success: false,
        error: error instanceof Error ? error.message : "Failed to read file",
      };
    }
  }
}

/**
 * Factory function to create a new filesystem handler
 * @param basePath Optional base path for file operations
 */
export async function createFSHandler(basePath?: string): Promise<FSHandler> {
  return new FSHandler();
}
