import fs from 'fs';
import path from 'path';

/**
 * Route structure for nginx generation
 */
export interface Route {
  path: string;
  methods: string[];
  params?: string[];
}

/**
 * Load routes from server-routes.json file
 */
export function loadRoutesFromFile(filePath: string): Route[] {
  try {
    const content = fs.readFileSync(filePath, 'utf8');
    const routes: Route[] = JSON.parse(content);
    return routes;
  } catch (error) {
    console.error(`Error loading routes from ${filePath}:`, error);
    return [];
  }
}

/**
 * Generate nginx location blocks from route definitions
 */
export function generateNginxRoutes(routes: Route[]): string {
  const locationBlocks: string[] = [];
  // Group routes by path to avoid overlapping location blocks
  const routesByPath = new Map<string, Set<string>>();

  routes.forEach(route => {
    const { path, methods } = route;

    // Remove /api prefix for nginx location blocks since main server strips it
    const nginxPath = path.startsWith('/api') ? path.substring(4) : path;

    if (!routesByPath.has(nginxPath)) {
      routesByPath.set(nginxPath, new Set());
    }

    // Add all methods for this path
    methods.forEach(method => {
      routesByPath.get(nginxPath)!.add(method.toUpperCase());
    });
  });

  // Generate location blocks for each unique path
  routesByPath.forEach((methods, nginxPath) => {
    const methodsArray = Array.from(methods);

    // Convert Express-style parameters (:id) to nginx regex format
    const nginxPathRegex = convertExpressPathToNginx(nginxPath);

    // Determine if we should use exact match or regex
    const hasParameters = nginxPath.includes(':');
    const locationDirective = hasParameters
      ? `location ~ ^${nginxPathRegex}$`
      : `location = ${nginxPath}`;

    // Generate method restriction using if statement (compatible with included location blocks)
    const methodRestriction =
      methodsArray.length > 0
        ? `    # Only allow specific HTTP methods
    if ($request_method !~ ^(${methodsArray.join('|')})$) {
        add_header Allow "${methodsArray.join(', ')}" always;
        return 405;
    }
    `
        : '';

    const locationBlock = `# Route: ${methodsArray.join(', ')} /api${nginxPath}
${locationDirective} {
${methodRestriction}
    # Proxy to the custom server - add /api prefix back since Express strips it
    proxy_pass http://127.0.0.1:\${port}/api$request_uri;
    
    # Standard proxy headers
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    
    # Preserve original request path structure
    proxy_set_header X-Original-URI $request_uri;
    proxy_set_header X-Original-Path $uri;
    
    # Essential settings for POST requests and request body handling
    proxy_buffering off;
    proxy_request_buffering off;
    proxy_http_version 1.1;
    proxy_set_header Connection "";
    
    # Timeout settings to prevent hanging
    proxy_connect_timeout 30s;
    proxy_send_timeout 30s;
    proxy_read_timeout 30s;
    
    # Handle potential errors gracefully
    proxy_intercept_errors on;
    error_page 502 503 504 /50x.html;
}`;
    locationBlocks.push(locationBlock);
  });
  return locationBlocks.join('\n\n');
}

/**
 * Generate nginx location blocks and write to file
 */
export function generateNginxConfigFromFile(
  routesFilePath: string,
  outputPath: string,
  port: number,
  bundleName: string,
  bundlePath: string
): void {
  const routes = loadRoutesFromFile(routesFilePath);

  if (routes.length === 0) {
    console.warn('No routes found or error loading routes file');
    return;
  }

  const nginxLocationBlocks = generateNginxRoutes(routes);

  // Create a complete nginx server block with the generated location blocks
  const nginxConfig = `# Nginx configuration template for custom bundle servers
# Auto-generated from server-routes.json
# Bundle: ${bundleName}
# Port: ${port}
# Path: ${bundlePath}
# Generated API routes from server-routes.json
${nginxLocationBlocks}
`;

  // Ensure output directory exists
  const outputDir = path.dirname(outputPath);
  if (!fs.existsSync(outputDir)) {
    fs.mkdirSync(outputDir, { recursive: true });
  }

  fs.writeFileSync(outputPath, nginxConfig);
  console.log(`Generated nginx config for ${routes.length} routes`);
  console.log(`Output: ${outputPath}`);
}

/**
 * Convert Express-style path to nginx regex pattern
 * Examples:
 * - /users -> /users
 * - /users/:id -> /users/([^/]+)
 * - /users/:id/posts/:postId -> /users/([^/]+)/posts/([^/]+)
 */
function convertExpressPathToNginx(expressPath: string): string {
  return expressPath
    .replace(/:[^/]+/g, '([^/]+)') // Convert :param to regex capture group
    .replace(/\//g, '\\/') // Escape forward slashes for regex
    .replace(/\./g, '\\.'); // Escape dots for regex
}
