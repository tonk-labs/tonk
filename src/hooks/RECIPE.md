# How to create hooks
## Core Principles
1. Type Safety
- Always define and export TypeScript interfaces for parameters and return types
- Use generics when hooks can work with different data types
- Use discriminated unions for complex state handling
- Include comprehensive JSDoc documentation for all types

2. Atomicity
- Each hook should do one thing and do it well
- Break complex operations into multiple composable hooks
- Avoid side effects that aren't directly related to the hook's primary purpose
- Make hooks as stateless as possible to improve reusability

3. Async Handling
- Return Promises with explicit type annotations
- Include proper error handling and type-safe error states
- Consider loading states in return types
- Provide abort/cleanup mechanisms for long-running operations

4. Data Exposure
- Return granular data that can be composed by consumers
- Include metadata that might be useful for debugging or logging
- Expose internal states when they might be useful to consumers
- Consider returning tuple types for hooks that have multiple related values

## Best Practices
### Type Definitions
```tsx
/**
 * Always start with clear interface definitions that document
 * the shape of your data and options
 */
export interface HookOptions<T> {
  /** Initial value for the operation */
  initial?: T;
  /** Timeout in milliseconds */
  timeout?: number;
  /** Optional configuration object */
  config?: {
    retry?: boolean;
    retryCount?: number;
  };
}

export interface HookResult<T> {
  /** The main data returned by the hook */
  data: T;
  /** Loading state */
  isLoading: boolean;
  /** Error state */
  error: Error | null;
  /** Metadata about the operation */
  metadata: {
    startTime: Date;
    attempts: number;
  };
}
```

### Hook Implementation
```tsx
/**
 * Template for a well-structured async hook
 * @template T - The type of data being handled
 * @param options - Configuration options for the hook
 * @returns Promise containing the operation result
 */
export const useAsyncOperation = async <T>(
  options: HookOptions<T>
): Promise<HookResult<T>> => {
  // Destructure options with defaults
  const {
    initial,
    timeout = 5000,
    config = { retry: false, retryCount: 3 }
  } = options;

  // Include abort controller for cleanup
  const controller = new AbortController();

  try {
    // Core logic here
    const result = await performOperation<T>(initial, {
      signal: controller.signal,
      timeout
    });

    return {
      data: result,
      isLoading: false,
      error: null,
      metadata: {
        startTime: new Date(),
        attempts: 1
      }
    };
  } catch (error) {
    // Type-safe error handling
    return {
      data: initial as T,
      isLoading: false,
      error: error instanceof Error ? error : new Error(String(error)),
      metadata: {
        startTime: new Date(),
        attempts: 1
      }
    };
  } finally {
    // Cleanup
    controller.abort();
  }
};
```

### Composition Example
```tsx
/**
 * Example of composing multiple atomic hooks
 */
export const useComplexOperation = async <T>(data: T) => {
  // Compose multiple atomic hooks
  const identity = await useCreateIdentity();
  const validation = await useValidateData(data);
  const storage = await usePersistData(data, identity.id);

  return {
    ...identity,
    ...validation,
    ...storage,
    // Add any additional combined state
    combinedMetadata: {
      totalTime: Date.now() - identity.metadata.startTime.getTime(),
      successful: !validation.error && !storage.error
    }
  };
};
```

## Examples
### createIdentity
```tsx
/**
 * Represents the structure of an identity in the system
 * @property id - Unique identifier for the identity
 * @property createdAt - Timestamp when the identity was created
 * @property updatedAt - Timestamp when the identity was last updated
 * @property version - Version number of the identity
 */
export interface Identity {
  id: string;
  createdAt: Date;
  updatedAt: Date;
  version: number;
}

/**
 * Configuration options for identity creation
 * @property prefix - Optional prefix for the generated ID
 * @property length - Length of the generated ID (excluding prefix)
 */
export interface IdentityOptions {
  prefix?: string;
  length?: number;
}

/**
 * Result of an identity creation operation
 * @property identity - The created identity object
 * @property raw - Raw data used to create the identity
 * @property metadata - Additional metadata about the creation process
 */
export interface IdentityResult {
  identity: Identity;
  raw: {
    timestamp: number;
    randomBytes: string;
  };
  metadata: {
    generatedAt: Date;
    source: string;
  };
}

const createIdentity = async (): Promise<IdentityResult> => {
    const timestamp = Date.now();
    const randomBytes = crypto.randomUUID().replace(/-/g, '').slice(0, length);
    const id = prefix ? `${prefix}_${randomBytes}` : randomBytes;

    const identity: Identity = {
      id,
      createdAt: new Date(timestamp),
      updatedAt: new Date(timestamp),
      version: 1
    };

    return {
      identity,
      raw: {
        timestamp,
        randomBytes
      },
      metadata: {
        generatedAt: new Date(),
        source: 'identity-system'
      }
    } as IdentityResult;
};
```
