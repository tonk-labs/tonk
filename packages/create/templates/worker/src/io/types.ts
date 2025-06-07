import { readDoc, writeDoc } from '@tonk/keepsync'
/**
 * Generic result wrapper for all I/O operations
 */
export interface IOResult<T> {
  success: boolean;
  data?: T;
  error?: string;
}

/**
 * Generic I/O handler interface
 * P = Parameters type for opening connections
 * I = Info type
 * T = Data type for read/write operations
 */
export interface IOHandler<P = any, I = any, T = any> {
  /**
   * Optional initialization function
   */
  init?(params?: P): Promise<void>;
  
  /**
   * Optional info module that returns relevant internal information
   * about the handler
   * 
   * It could be credentials, file path, etc. It's just a way to introspect state.
   */
  info?(): Promise<I>
  
  /**
   * Read data from the specified location
   */
  read(location: string): Promise<IOResult<T>>;
}

/**
 * I/O Manager that orchestrates different handlers
 */
export interface IOManager {
  /**
   * Register a handler for a specific scheme (fs:, http:, keepsync:)
   */
  register(scheme: string, handler: IOHandler): void;
  
  /**
   * Get data from any registered handler based on URL scheme
   */
  read(url: string): Promise<IOResult<any>>;
}

/**
 * Interface for data mapper functions
 */
export interface DataMapper<S, T> {
  //Schema S is the full document data structure 
  //Data T is the type of the IOResult 
  (state: S, data: T): S;
}

/**
 * IOResult to Keepsync document mapper
 * Takes IOResult data and writes it to keepsync using a mapper function
 */
export class IOResultMapper<S = any, T = any> {
  /**
   * Maps IOResult data and writes to keepsync document
   * @param result The IOResult to process
   * @param path The keepsync document path
   * @param mapper Function to transform the data before writing
   * @returns Promise resolving to success status
   */
  async mapAndWrite(
    path: string,
    result: IOResult<T>,
    mapper: DataMapper<S, T>
  ): Promise<IOResult<void>> {
    try {
      if (!result.success || !result.data) {
        return {
          success: false,
          error: result.error || "No data to write"
        };
      }

      //we just assume it's an object
      const data = await readDoc<S>(path);

      // Transform the data using the mapper function
      const newData = mapper(data, result.data);
      
      // Write to keepsync
      await writeDoc(path, newData);
      
      return { success: true };
    } catch (error) {
      return {
        success: false,
        error: error instanceof Error ? error.message : "Failed to write to keepsync"
      };
    }
  }
}

