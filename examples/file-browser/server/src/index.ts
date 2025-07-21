import cors from "cors";
import { fileURLToPath } from "url";
import { dirname, join } from "path";
import fs from "fs";
import dotenv from "dotenv";
import { ExpressWithRouteTracking } from "./routeTracker.js";

// Import configuration
const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const projectRoot = join(__dirname, "..", "..");

// Load environment variables from .env file in project root
dotenv.config({ path: join(projectRoot, ".env") });

// NOTE: if you do not use ExpressWithRouteTracking, the endpoints will break. This is very important.
// You MUST use ExpressWithRouteTracking!
const app = new ExpressWithRouteTracking();
const PORT = process.env.PORT ? parseInt(process.env.PORT, 10) : 6080;

// Enable CORS
app.use(cors());

// Add ping endpoint for health checks
// WARNING: ALL SERVERS MUST INCLUDE A /ping ENDPOINT FOR HEALTH CHECKS, OTHERWISE THEY WILL FAIL
app.get("/ping", (_req, res) => {
  res.status(200).send("OK");
});

// Check if --routes CLI parameter is provided
const hasRoutesParam = process.argv.includes("--routes");

// Start the server only if --routes parameter is not provided
if (!hasRoutesParam) {
  app.listen(PORT, () => {
    console.log(`Server running on port ${PORT}`);
  });
} else {
  // Output routes in JSON format for nginx generation using tracked routes
  const trackedRoutes = app.getRoutes();
  const routes = trackedRoutes.map((route) => ({
    path: route.path,
    methods:
      route.method === "ALL"
        ? ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"]
        : [route.method],
    ...(route.params && { params: route.params }),
  }));

  // Write routes to file for nginx generation
  const routesFilePath = join(__dirname, "..", "server-routes.json");
  fs.writeFileSync(routesFilePath, JSON.stringify(routes, null, 2));
  console.log(`Routes written to ${routesFilePath}`);
}
