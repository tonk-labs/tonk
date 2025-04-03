# @tonk/integration-template

A dual-purpose TypeScript library for data fetching that works in both Node.js and browser environments.

## Features

- 🌍 Universal - works in Node.js and browsers
- 🔄 Promise-based API
- ⚡ Lightweight and tree-shakeable
- 🔒 TypeScript support out of the box
- ⏱️ Configurable timeouts
- 🚦 Automatic error handling
- 🔌 Customizable base URL and default options

## Installation

```bash
npm install @tonk/integration-template
# or
yarn add @tonk/integration-template
# or
pnpm add @tonk/integration-template
```

## Usage

```typescript
import { FetchClient } from "@tonk/integration-template";

// Create a client instance
const client = new FetchClient({
  baseUrl: "https://api.example.com",
  timeout: 5000, // 5 seconds
});

// GET request
const { data, error } = await client.get<User>("/users/1");
if (error) {
  console.error("Error fetching user:", error);
} else {
  console.log("User data:", data);
}

// POST request
const response = await client.post<User>("/users", {
  name: "John Doe",
  email: "john@example.com",
});

// PUT request
await client.put<User>("/users/1", {
  name: "Jane Doe",
});

// DELETE request
await client.delete("/users/1");
```

## API

### `FetchClient`

The main class for making HTTP requests.

#### Constructor

```typescript
new FetchClient(options?: FetchOptions)
```

##### Options

- `baseUrl?: string` - Base URL for all requests
- `timeout?: number` - Default timeout in milliseconds
- ...all other fetch options

#### Methods

##### `get<T>(path: string, options?: FetchOptions): Promise<FetchResponse<T>>`

Make a GET request.

##### `post<T>(path: string, body?: unknown, options?: FetchOptions): Promise<FetchResponse<T>>`

Make a POST request.

##### `put<T>(path: string, body?: unknown, options?: FetchOptions): Promise<FetchResponse<T>>`

Make a PUT request.

##### `delete<T>(path: string, options?: FetchOptions): Promise<FetchResponse<T>>`

Make a DELETE request.

### Types

#### `FetchOptions`

Extends the standard `RequestInit` interface with additional options:

```typescript
interface FetchOptions extends RequestInit {
  baseUrl?: string;
  timeout?: number;
}
```

#### `FetchResponse<T>`

The response type for all requests:

```typescript
interface FetchResponse<T> {
  data: T | null;
  error: Error | null;
  status: number;
  headers: Headers;
}
```

## License

MIT
