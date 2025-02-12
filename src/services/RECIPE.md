# How to create services
- Keep functions small and focused
    - Each function should do one thing well
    - Extract reusable logic into helper functions
    - Aim for functions under 20 lines
- Use strong typing
    - Avoid `any` and `unknown` where possible
    - Define explicit interfaces for all data structures
    - Use union types for specific value sets
- Handle edge cases
    - Validate input parameters
    - Handle network timeouts
    - Include retry logic for transient failures
    - Transform API errors into application-specific errors
- Configuration management
    - Use environment variables for sensitive data
    - Provide defaults for optional config
    - Validate configuration at service creation

## File structure
Each file should:
- Be focused on a single external service or API
- Export a clear interface for interacting with that service
- Include comprehensive type definitions
- Handle errors gracefully
- Include detailed JSDoc documentation

### Example File Layout
```tsx
// types at the top
// configuration interfaces
// error classes
// helper functions
// main service functions
// exports
```

## Type Definitions
Always define explicit types for:
- API request parameters
- Response data structures
- Configuration options
- Error states

### Example
```tsx
/** Configuration for connecting to the external service */
interface ServiceConfig {
    /** Base URL for the API */
    baseUrl: string;
    /** API key for authentication */
    apiKey: string;
    /** Timeout in milliseconds */
    timeout: number;
}

/** Response structure for user data */
interface UserResponse {
    /** Unique identifier */
    id: string;
    /** User's email address */
    email: string;
    /** ISO 8601 timestamp of last update */
    updatedAt: string;
}
```

## Error Handling
Create specific error classes:
```tsx
export class ServiceError extends Error {
    constructor(
        message: string,
        public statusCode?: number,
        public rawError?: unknown
    ) {
        super(message);
        this.name = 'ServiceError';
    }
}
```

## Documentation
Use JSDoc comments for all exported functions:
```tsx
/**
 * Fetches user data from the external service
 * 
 * @param userId - Unique identifier for the user
 * @param options - Optional configuration overrides
 * @throws {ServiceError} When the API request fails
 * @returns The user's data
 * 
 * @example
 * const user = await getUser('123', { timeout: 5000 });
 * console.log(user.email);
 */
```

## Service Creation Pattern
Use a factory pattern for service creation:
```tsx
export function createUserService(config: ServiceConfig) {
    const headers = {
        'Authorization': `Bearer ${config.apiKey}`,
        'Content-Type': 'application/json',
    };

    return {
        getUser: async (id: string) => {
            // implementation
        },
        updateUser: async (id: string, data: UpdateUserData) => {
            // implementation
        },
    };
}
```

## Examples
### Payment Service
```tsx
/** Configuration for the payment service */
interface PaymentServiceConfig {
    apiKey: string;
    baseUrl: string;
    timeout?: number;
}

/** Payment creation parameters */
interface CreatePaymentParams {
    amount: number;
    currency: string;
    description: string;
}

/** Payment response data */
interface PaymentResponse {
    id: string;
    status: 'pending' | 'completed' | 'failed';
    amount: number;
    currency: string;
    createdAt: string;
}

/** Creates a configured payment service instance */
export function createPaymentService(config: PaymentServiceConfig) {
    const headers = {
        'Authorization': `Bearer ${config.apiKey}`,
        'Content-Type': 'application/json',
    };

    /**
     * Creates a new payment
     * 
     * @throws {ServiceError} When payment creation fails
     */
    async function createPayment(
        params: CreatePaymentParams
    ): Promise<PaymentResponse> {
        const response = await fetch(`${config.baseUrl}/payments`, {
            method: 'POST',
            headers,
            body: JSON.stringify(params),
            signal: AbortSignal.timeout(config.timeout ?? 5000),
        });

        if (!response.ok) {
            throw new ServiceError(
                'Payment creation failed',
                response.status
            );
        }

        return response.json();
    }

    return {
        createPayment,
    };
}
```
