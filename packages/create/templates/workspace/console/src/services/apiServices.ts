export interface ApiService {
  prefix: string; // The route prefix (e.g., "weather")
  baseUrl: string; // The actual API base URL
  requiresAuth?: boolean; // Whether authentication is needed
  authType?: "bearer" | "apikey" | "basic" | "query"; // Authentication type
  authHeaderName?: string; // Header name for auth (e.g., "Authorization" or "X-API-Key")
  authEnvVar?: string; // API key or auth secret
  authQueryParamName?: string; // If using query auth type, the corresponding query param
  isLocal?: boolean; // Whether this is a local service handled by the server itself
}

// Register all your services here
export const apiServices: ApiService[] = [
];
