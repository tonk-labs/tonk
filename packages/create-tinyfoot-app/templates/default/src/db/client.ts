import { PrismaClient } from '@prisma/client'

/**
 * Global PrismaClient instance
 * @description This is the main database client that should be used throughout the application.
 * The LLM should use this to implement specific database operations.
 * 
 * Example implementation:
 * ```typescript
 * // Creating a user
 * const user = await prisma.user.create({
 *   data: {
 *     email: "user@example.com",
 *     // ... other user fields
 *   }
 * });
 * 
 * // Querying with relations
 * const userWithPosts = await prisma.user.findUnique({
 *   where: { id: userId },
 *   include: { posts: true }
 * });
 * ```
 */
declare global {
  var prisma: PrismaClient | undefined
}

export const prisma = global.prisma || new PrismaClient()

if (process.env.NODE_ENV !== 'production') {
  global.prisma = prisma
}
