import fs from "fs";
import path from "path";

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
    const content = fs.readFileSync(filePath, "utf8");
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
export function generateNginxRoutes(routes: Route[], port: number): string {
  const locationBlocks: string[] = [];

  routes.forEach((route) => {
    const { path, methods } = route;

    // Convert Express-style parameters (:id) to nginx regex format
    const nginxPath = convertExpressPathToNginx(path);

    // Generate method restriction
    const methodRestriction =
      methods.length > 0
        ? `        # Only allow specific HTTP methods
        if ($request_method !~ ^(${methods.join("|")})$) {
            return 405;
        }
        `
        : "";

    const locationBlock = `    # Route: ${methods.join(", ")} ${path}
    location ~ ^${nginxPath}$ {
${methodRestriction}
        # Proxy to the custom server
        proxy_pass http://127.0.0.1:${port}$request_uri;
        
        # Standard proxy headers
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        
        # Preserve original request path structure
        proxy_set_header X-Original-URI $request_uri;
        proxy_set_header X-Original-Path $uri;
        
        # Handle potential errors gracefully
        proxy_intercept_errors on;
        error_page 502 503 504 /50x.html;
    }`;

    locationBlocks.push(locationBlock);
  });

  return locationBlocks.join("\n\n");
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
    console.warn("No routes found or error loading routes file");
    return;
  }

  const nginxLocationBlocks = generateNginxRoutes(routes, port);

  // Create a complete nginx server block with the generated location blocks
  const nginxConfig = `# Nginx configuration template for custom bundle servers
# Auto-generated from server-routes.json
# Bundle: ${bundleName}
# Port: ${port}
# Path: ${bundlePath}

server {
    listen 8080;
    server_name _;
    
    # Generated API routes from server-routes.json
${nginxLocationBlocks}
}`;

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
 * - /api/users -> /api/users
 * - /api/users/:id -> /api/users/([^/]+)
 * - /api/users/:id/posts/:postId -> /api/users/([^/]+)/posts/([^/]+)
 */
function convertExpressPathToNginx(expressPath: string): string {
  return expressPath
    .replace(/:[^\/]+/g, "([^/]+)") // Convert :param to regex capture group
    .replace(/\//g, "\\/") // Escape forward slashes for regex
    .replace(/\./g, "\\."); // Escape dots for regex
}
