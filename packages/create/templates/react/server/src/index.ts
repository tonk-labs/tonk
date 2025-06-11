import express from "express";
import cors from "cors";
import { createProxyMiddleware } from "http-proxy-middleware";
import { fileURLToPath } from "url";
import { dirname, join } from "path";
import fs from "fs";
import dotenv from "dotenv";

interface ApiService {
  prefix: string; // The route prefix (e.g., "weather")
  baseUrl: string; // The actual API base URL
  requiresAuth?: boolean; // Whether authentication is needed
  authType?: "bearer" | "apikey" | "basic" | "query"; // Authentication type
  authHeaderName?: string; // Header name for auth (e.g., "Authorization" or "X-API-Key")
  authEnvVar?: string; // API key or auth secret
  authQueryParamName?: string; // If using query auth type, the corresponding query param
}

// Import API services configuration
const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const projectRoot = join(__dirname, "..", "..");

// Load environment variables from .env file in project root
dotenv.config({ path: join(projectRoot, ".env") });

// Read API services from the frontend project
const apiServicesPath = join(projectRoot, "src", "services", "apiServices.ts");
const apiServicesContent = fs.readFileSync(apiServicesPath, "utf-8");

// Extract API services using regex (a simple approach)
const servicesRegex =
  /export const apiServices: ApiService\[\] = \[([\s\S]*?)\];/;
const servicesMatch = apiServicesContent.match(servicesRegex);

if (!servicesMatch) {
  console.error("Could not parse API services from the frontend project");
  process.exit(1);
}

// Parse the services (this is a simplified approach)
const servicesString = servicesMatch[1];
const services = eval(`[${servicesString}]`);

const app = express();
const PORT = process.env.PORT || 6080;

// Enable CORS
app.use(cors());

// Setup API proxies based on the configuration
services.forEach((service: ApiService) => {
  const {
    prefix,
    baseUrl,
    requiresAuth,
    authType,
    authEnvVar,
    authHeaderName,
    authQueryParamName,
  } = service;

  // Create proxy middleware for this service
  app.use(
    `/api/${prefix}`,
    createProxyMiddleware({
      target: baseUrl,
      changeOrigin: true,
      pathRewrite: {
        [`^/api/${prefix}`]: "",
      },
      on: {
        proxyReq: (proxyReq, _req, _res) => {
          // Add authentication if required
          if (requiresAuth) {
            const authValue = process.env[authEnvVar!] || "";

            if (authType === "bearer") {
              proxyReq.setHeader(
                authHeaderName || "Authorization",
                `Bearer ${authValue}`,
              );
            } else if (authType === "apikey") {
              proxyReq.setHeader(authHeaderName || "X-API-Key", authValue);
            } else if (authType === "basic") {
              proxyReq.setHeader(
                authHeaderName || "Authorization",
                `Basic ${authValue}`,
              );
            } else if (authType === "query") {
              // For query params, we need to modify the URL
              const url = new URL(proxyReq.path, baseUrl);
              url.searchParams.set(authQueryParamName || "apikey", authValue);
              proxyReq.path = url.pathname + url.search;
            }
          }
          // Log the request
          console.log(`Proxying request to ${baseUrl}${proxyReq.path}`);
        },
      },
    }),
  );
});

// Start the server
app.listen(PORT, () => {
  console.log(`API proxy server running on port ${PORT}`);
  console.log(`Proxying the following services:`);
  services.forEach((service: ApiService) => {
    console.log(`- ${service.prefix} -> ${service.baseUrl}`);
  });
});
