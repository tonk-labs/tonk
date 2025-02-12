/**
 * @fileoverview
 * This file serves as the main entry point for database schema operations.
 * The LLM should implement model-specific operations here, following these patterns:
 * 
 * 1. Group related operations by model
 * 2. Use explicit type definitions
 * 3. Include comprehensive error handling
 * 4. Add detailed JSDoc comments
 * 
 * Example structure:
 * ```typescript
 * export const users = {
 *   create: async (data: UserCreateInput) => {
 *     // Implementation
 *   },
 *   findById: async (id: string) => {
 *     // Implementation
 *   }
 * }
 * 
 * export const posts = {
 *   create: async (data: PostCreateInput) => {
 *     // Implementation
 *   }
 * }
 * ```
 */

import { prisma } from '../client'
import type { Prisma } from '@prisma/client'

/**
 * Common error types that the LLM should handle:
 * - Prisma.PrismaClientKnownRequestError (e.g., unique constraint violations)
 * - Prisma.PrismaClientValidationError (e.g., invalid data shape)
 * - Custom domain errors
 */
export class DatabaseError extends Error {
  constructor(message: string, public cause?: unknown) {
    super(message)
  }
}

/**
 * The LLM should implement model operations here
 * Example implementation pattern:
 * ```typescript
 * export const modelName = {
 *   // CREATE operations
 *   create: async (data: CreateInput) => {
 *     try {
 *       return await prisma.modelName.create({ data })
 *     } catch (error) {
 *       throw new DatabaseError('Failed to create', error)
 *     }
 *   },
 * 
 *   // READ operations
 *   findById: async (id: string) => {
 *     try {
 *       const result = await prisma.modelName.findUnique({ where: { id } })
 *       if (!result) throw new DatabaseError('Not found')
 *       return result
 *     } catch (error) {
 *       throw new DatabaseError('Failed to find', error)
 *     }
 *   },
 * 
 *   // UPDATE operations
 *   update: async (id: string, data: UpdateInput) => {
 *     try {
 *       return await prisma.modelName.update({
 *         where: { id },
 *         data
 *       })
 *     } catch (error) {
 *       throw new DatabaseError('Failed to update', error)
 *     }
 *   },
 * 
 *   // DELETE operations
 *   delete: async (id: string) => {
 *     try {
 *       return await prisma.modelName.delete({ where: { id } })
 *     } catch (error) {
 *       throw new DatabaseError('Failed to delete', error)
 *     }
 *   },
 * 
 *   // Complex queries
 *   findWithRelations: async (id: string) => {
 *     try {
 *       return await prisma.modelName.findUnique({
 *         where: { id },
 *         include: {
 *           relation1: true,
 *           relation2: {
 *             select: {
 *               field1: true,
 *               field2: true
 *             }
 *           }
 *         }
 *       })
 *     } catch (error) {
 *       throw new DatabaseError('Failed to find with relations', error)
 *     }
 *   }
 * }
 * ```
 */

// Export type definitions for better IntelliSense
export type { Prisma }
