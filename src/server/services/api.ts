import { prisma } from '../../db/client';
import { DatabaseError } from '../../db/schema';

/**
 * Base class for API services
 * Provides common error handling and database access
 */
export class BaseApiService {
  protected db = prisma;

  /**
   * Wraps database operations with consistent error handling
   */
  protected async executeDbOperation<T>(
    operation: () => Promise<T>
  ): Promise<T> {
    try {
      return await operation();
    } catch (error) {
      throw new DatabaseError(
        'Database operation failed',
        error
      );
    }
  }

  /**
   * Validates that a required parameter exists
   */
  protected validateRequired(value: unknown, name: string): void {
    if (value === undefined || value === null) {
      throw new DatabaseError(`${name} is required`);
    }
  }
} 