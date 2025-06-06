import { readDoc, writeDoc, ls, mkDir, rm, DocNode, DirNode } from "@tonk/keepsync";

/**
 * Service for interacting with keepsync file system
 */
export const KeepsyncService = {
  /**
   * Read a document from keepsync
   * @param path Path to the document
   * @returns Document content or undefined if not found
   */
  async readDocument<T>(path: string): Promise<T | undefined> {
    try {
      return await readDoc<T>(path);
    } catch (error) {
      console.error("Error reading document:", error);
      throw error;
    }
  },

  /**
   * Write a document to keepsync
   * @param path Path to the document
   * @param content Content to write
   */
  async writeDocument<T>(path: string, content: T): Promise<void> {
    try {
      await writeDoc<T>(path, content);
    } catch (error) {
      console.error("Error writing document:", error);
      throw error;
    }
  },

  /**
   * List contents of a directory
   * @param path Path to the directory
   * @returns Directory node or undefined if not found
   */
  async listDirectory(path: string): Promise<DocNode | undefined> {
    try {
      return await ls(path);
    } catch (error) {
      console.error("Error listing directory:", error);
      throw error;
    }
  },

  /**
   * Create a directory
   * @param path Path to create the directory
   * @returns Created directory node or undefined if creation failed
   */
  async createDirectory(path: string): Promise<DirNode | undefined> {
    try {
      return await mkDir(path);
    } catch (error) {
      console.error("Error creating directory:", error);
      throw error;
    }
  },

  /**
   * Remove a document or directory
   * @param path Path to the document or directory
   * @returns True if removal was successful
   */
  async removeItem(path: string): Promise<boolean> {
    try {
      return await rm(path);
    } catch (error) {
      console.error("Error removing item:", error);
      throw error;
    }
  },

  /**
   * Get file name from path
   * @param path Full path
   * @returns File name
   */
  getFileName(path: string): string {
    const parts = path.split("/");
    return parts[parts.length - 1];
  },

  /**
   * Get parent directory path
   * @param path Current path
   * @returns Parent directory path
   */
  getParentPath(path: string): string {
    const parts = path.split("/");
    parts.pop();
    return parts.join("/") || "/";
  },

  /**
   * Join path segments
   * @param base Base path
   * @param segment Path segment to add
   * @returns Joined path
   */
  joinPath(base: string, segment: string): string {
    if (base === "/") {
      return `/${segment}`;
    }
    return `${base}/${segment}`;
  },

  /**
   * Format timestamp to readable date
   * @param timestamp Timestamp in milliseconds
   * @returns Formatted date string
   */
  formatDate(timestamp: number): string {
    return new Date(timestamp).toLocaleString();
  }
};
