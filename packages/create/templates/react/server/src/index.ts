import express from "express";
import { createProxyMiddleware } from "http-proxy-middleware";
import cors from "cors";
import dotenv from "dotenv";

// Load environment variables from .env file
dotenv.config();

const app = express();
const PORT = process.env.PORT || 6080;

// Enable CORS for all routes
app.use(cors());

// Parse JSON request bodies
app.use(express.json());

// Basic route for hello world
app.get("/api/hello", (_req, res) => {
  res.send("Hello World Api!");
});

// Health check endpoint
app.get("/ping", (_req, res) => {
  res.status(200).send("OK");
});

/**
 * API Proxy Middleware
 *
 * This middleware proxies requests from /api/:endpoint to external services
 * while keeping API keys and credentials secure on the server side.
 *
 * Example: A request to /api/weather will be handled by the proxy
 * and forwarded to the appropriate external service.
 */

// Generic API proxy handler
app.use("/api/:endpoint", (req, res, next) => {
  const endpoint = req.params.endpoint;

  // Skip if this is a direct API endpoint we've defined
  if (endpoint === "hello") {
    return next();
  }

  // Handle different API endpoints
  switch (endpoint) {
    case "weather":
      // Example: Proxy to a weather API
      const weatherApiKey = process.env.WEATHER_API_KEY;
      if (!weatherApiKey) {
        return res
          .status(500)
          .json({ error: "Weather API key not configured" });
      }

      // Create a proxy for this specific request
      createProxyMiddleware({
        target: "https://api.weatherapi.com/v1",
        changeOrigin: true,
        pathRewrite: {
          [`^/api/weather`]: "",
        },
        on: {
          proxyReq: (proxyReq) => {
            // Add API key to the request
            proxyReq.path = `${proxyReq.path}${proxyReq.path.includes("?") ? "&" : "?"}key=${weatherApiKey}`;
          },
        },
      })(req, res, next);
      break;

    // Add more API endpoints as needed

    default:
      // If no specific handler is defined, return a 404
      res.status(404).json({ error: `API endpoint '${endpoint}' not found` });
  }
});

// Start the server
app.listen(PORT, () => {
  console.log(`Server running on port ${PORT}`);
  console.log(`API proxy available at http://localhost:${PORT}/api/:endpoint`);
});
