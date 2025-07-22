import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";
import { execSync } from "child_process";
import { generateNginxConfigFromFile } from "./nginxRouteGenerator.js";

// Get the directory name of the current module
const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const projectRoot = path.resolve(__dirname, "..");

const bundleDir = path.resolve(projectRoot, "bundle");
const serverDir = path.resolve(projectRoot, "server");

// Read package.json to get the project name
const packageJsonPath = path.resolve(projectRoot, "package.json");
const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));
const bundleName = packageJson.name || "tonk-app";

// Clean and create bundle directory
if (fs.existsSync(bundleDir)) {
  fs.rmSync(bundleDir, { recursive: true, force: true });
}
fs.mkdirSync(bundleDir, { recursive: true });

console.log("Building frontend...");
// Build frontend with Vite
execSync("vite build", { cwd: projectRoot, stdio: "inherit" });

console.log("Generating server routes...");
// Generate server routes for nginx configuration
execSync("tsx server/src/index.ts --routes", {
  cwd: projectRoot,
  stdio: "inherit",
});

console.log("Building server...");
// Build server
execSync("pnpm build", { cwd: serverDir, stdio: "inherit" });

console.log("Creating bundle structure...");

// Copy frontend dist to bundle/dist
const frontendDistSrc = path.resolve(projectRoot, "dist");
const frontendDistDest = path.resolve(bundleDir, "dist");
if (fs.existsSync(frontendDistSrc)) {
  fs.cpSync(frontendDistSrc, frontendDistDest, { recursive: true });
}

// Copy server files to bundle/server
const serverBundleDir = path.resolve(bundleDir, "server");
fs.mkdirSync(serverBundleDir, { recursive: true });

// Copy server dist
const serverDistSrc = path.resolve(serverDir, "dist");
const serverDistDest = path.resolve(serverBundleDir, "dist");
if (fs.existsSync(serverDistSrc)) {
  fs.cpSync(serverDistSrc, serverDistDest, { recursive: true });
}

// Copy server package.json
const serverPackageJsonSrc = path.resolve(serverDir, "package.json");
const serverPackageJsonDest = path.resolve(serverBundleDir, "package.json");
if (fs.existsSync(serverPackageJsonSrc)) {
  fs.copyFileSync(serverPackageJsonSrc, serverPackageJsonDest);
}

// Copy server-routes.json
const serverRoutesSrc = path.resolve(serverDir, "server-routes.json");
const serverRoutesDest = path.resolve(serverBundleDir, "server-routes.json");
if (fs.existsSync(serverRoutesSrc)) {
  fs.copyFileSync(serverRoutesSrc, serverRoutesDest);
}

// Generate nginx config from server routes
console.log("Generating nginx configuration...");
const routesFilePath = path.resolve(
  projectRoot,
  "server",
  "server-routes.json",
);
const nginxConfigDest = path.resolve(serverBundleDir, `app-${bundleName}.conf`);

if (fs.existsSync(routesFilePath)) {
  // Use a default port that can be overridden by the server management system
  const defaultPort = 6080;
  const bundlePath = `/bundles/${bundleName}`;

  generateNginxConfigFromFile(
    routesFilePath,
    nginxConfigDest,
    defaultPort,
    bundleName,
    bundlePath,
  );
} else {
  console.warn(
    "server-routes.json not found, skipping nginx config generation",
  );
}

// API services configuration has been removed from this system

console.log("Bundle created successfully!");
console.log("Bundle structure:");
console.log("bundle/");
console.log("├── dist/                    # Frontend files");
console.log("├── server/                  # Custom server");
console.log("│   ├── dist/               # Built server code (JS)");
console.log("│   ├── package.json");
console.log("│   ├── server-routes.json  # API routes definition");
console.log(`│   └── app-${bundleName}.conf   # Generated nginx config`);
