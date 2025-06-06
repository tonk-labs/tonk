# API Services Configuration Guide

This document explains how to configure and use API services in your frontend Tonk app. The system provides a flexible way to define external API endpoints and handle authentication, which are then proxied through a backend server.

## Table of Contents

- [Overview](#overview)
- [API Service Configuration](#api-service-configuration)
- [Adding a New API Service](#adding-a-new-api-service)
- [Using API Services in Frontend Components](#using-api-services-in-frontend-components)
- [Authentication Methods](#authentication-methods)
- [Server-Side Proxy](#server-side-proxy)
- [Environment Variables](#environment-variables)
- [Examples](#examples)

## Overview

The application uses a proxy-based approach to handle external API requests. This provides several benefits:

- Hides API keys and secrets from the frontend
- Avoids CORS issues
- Provides a consistent API interface for frontend components
- Centralizes API configuration

## API Service Configuration

API services are defined in `src/services/apiServices.ts`. Each service is configured with the following properties:

```typescript
export interface ApiService {
  prefix: string;         // The route prefix (e.g., "weather")
  baseUrl: string;        // The actual API base URL
  requiresAuth?: boolean; // Whether authentication is needed
  authType?: "bearer" | "apikey" | "basic" | "query"; // Authentication type
  authHeaderName?: string; // Header name for auth (e.g., "Authorization" or "X-API-Key")
  authEnvVar?: string;    // API key or auth secret environment variable name
  authQueryParamName?: string; // If using query auth type, the corresponding query param
}
```

## Adding a New API Service

To add a new API service:

1. Open `src/services/apiServices.ts`
2. Add a new entry to the `apiServices` array:

```typescript
export const apiServices: ApiService[] = [
  // Existing services...
  {
    prefix: "my-service",
    baseUrl: "https://api.example.com/v1",
    requiresAuth: true,
    authType: "bearer",
    authHeaderName: "Authorization",
    authEnvVar: "MY_SERVICE_API_KEY",
  },
];
```

## Using API Services in Frontend Components

Once configured, you can use the API service in your frontend components by making requests to the proxy endpoint:

```typescript
// Example: Fetching data from a configured API service
const fetchData = async () => {
  try {
    // The format is /api/{prefix}/{endpoint}?{query-params}
    const response = await fetch(`/api/my-service/resource?param=value`);
    
    if (!response.ok) {
      throw new Error(`Error: ${response.status} ${response.statusText}`);
    }
    
    const data = await response.json();
    // Process the data...
  } catch (error) {
    // Handle errors...
  }
};
```

## Authentication Methods

The system supports four authentication methods:

1. **Bearer Token** (`bearer`): Adds an `Authorization: Bearer {token}` header
2. **API Key** (`apikey`): Adds a custom header (e.g., `X-API-Key: {key}`)
3. **Basic Auth** (`basic`): Adds an `Authorization: Basic {credentials}` header
4. **Query Parameter** (`query`): Adds the auth token as a query parameter

## Server-Side Proxy

The server-side proxy automatically reads the API service configuration from the frontend and sets up the appropriate proxy middleware. The proxy:

1. Forwards requests to the appropriate API endpoint
2. Adds authentication headers or parameters as needed
3. Handles CORS and other cross-origin issues
4. Logs requests for debugging purposes

## Environment Variables

In production, API keys and secrets should be stored as environment variables. The `authEnvVar` property specifies which environment variable contains the authentication token.

For local development, you can:

1. Create a `.env` file in the server directory
2. Add your API keys in the format `KEY=value`
3. Reference these keys in your API service configuration

## Examples

### Weather API Example

```typescript
// In apiServices.ts
export const apiServices: ApiService[] = [
  {
    prefix: "weather",
    baseUrl: "https://api.openweathermap.org/data/2.5",
    requiresAuth: true,
    authType: "query",
    authEnvVar: "OPENWEATHER_API_KEY",
    authQueryParamName: "appid",
  },
];

// In a React component
const fetchWeather = async (city) => {
  try {
    const response = await fetch(
      `/api/weather/weather?q=${encodeURIComponent(city)}&units=metric`
    );
    
    if (!response.ok) {
      throw new Error(`Error: ${response.status} ${response.statusText}`);
    }
    
    const data = await response.json();
    // Process weather data...
  } catch (error) {
    // Handle errors...
  }
};
```

### REST API with Bearer Token Example

```typescript
// In apiServices.ts
export const apiServices: ApiService[] = [
  {
    prefix: "users",
    baseUrl: "https://api.example.com/users",
    requiresAuth: true,
    authType: "bearer",
    authEnvVar: "USER_API_TOKEN",
  },
];

// In a React component
const fetchUserProfile = async (userId) => {
  try {
    const response = await fetch(`/api/users/${userId}/profile`);
    
    if (!response.ok) {
      throw new Error(`Error: ${response.status} ${response.statusText}`);
    }
    
    const data = await response.json();
    // Process user data...
  } catch (error) {
    // Handle errors...
  }
};
```
