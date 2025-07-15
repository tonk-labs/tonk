import cors from "cors";
import express, { Request, Response, NextFunction } from "express";
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

// Parse JSON bodies
app.use(express.json());
app.use(express.urlencoded({ extended: true }));

// Add request logging middleware
app.use((req: Request, _res: Response, next: NextFunction) => {
  console.log(`${req.method} ${req.url}`);
  next();
});

// Add ping endpoint for health checks
// WARNING: ALL SERVERS MUST INCLUDE A /ping ENDPOINT FOR HEALTH CHECKS, OTHERWISE THEY WILL FAIL
app.get("/ping", (_req, res) => {
  res.status(200).send("OK");
});

// Sample API endpoints for testing proxy functionality
app.get("/api/users", (_req, res) => {
  const users = [
    { id: 1, name: "John Doe", email: "john@example.com", role: "admin" },
    { id: 2, name: "Jane Smith", email: "jane@example.com", role: "user" },
    { id: 3, name: "Bob Johnson", email: "bob@example.com", role: "user" },
  ];
  res.json({ success: true, data: users });
});

app.get("/api/users/:id", (req, res) => {
  const userId = parseInt(req.params.id);
  const users = [
    {
      id: 1,
      name: "John Doe",
      email: "john@example.com",
      role: "admin",
      lastLogin: "2024-01-15",
    },
    {
      id: 2,
      name: "Jane Smith",
      email: "jane@example.com",
      role: "user",
      lastLogin: "2024-01-14",
    },
    {
      id: 3,
      name: "Bob Johnson",
      email: "bob@example.com",
      role: "user",
      lastLogin: "2024-01-13",
    },
  ];

  const user = users.find((u) => u.id === userId);
  if (user) {
    res.json({ success: true, data: user });
  } else {
    res.status(404).json({ success: false, error: "User not found" });
  }
});

app.post("/api/users", (req, res) => {
  const newUser = {
    id: Date.now(),
    name: req.body?.name || "New User",
    email: req.body?.email || "new@example.com",
    role: req.body?.role || "user",
    created: new Date().toISOString(),
  };
  res.status(201).json({
    success: true,
    data: newUser,
    message: "User created successfully",
  });
});

app.get("/api/stats", (_req, res) => {
  const stats = {
    totalUsers: 3,
    activeUsers: 2,
    totalRequests: Math.floor(Math.random() * 1000) + 100,
    uptime: "2 days, 4 hours",
    serverTime: new Date().toISOString(),
    version: "1.0.0",
  };
  res.json({ success: true, data: stats });
});

app.get("/api/health", (_req, res) => {
  const health = {
    status: "healthy",
    database: "connected",
    memory: `${Math.floor(Math.random() * 50) + 20}%`,
    cpu: `${Math.floor(Math.random() * 30) + 10}%`,
    timestamp: new Date().toISOString(),
  };
  res.json({ success: true, data: health });
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
