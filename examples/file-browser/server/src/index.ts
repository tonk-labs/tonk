import express from "express";
import cors from "cors";
import { fileURLToPath } from "url";
import { dirname, join } from "path";

// Import configuration
const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const projectRoot = join(__dirname, "..", "..");

const app = express();
const PORT = process.env.PORT || 6080;

// Enable CORS
app.use(cors());

console.log(`API proxy functionality has been removed from this system.`);

// Start the server
app.listen(PORT, () => {
  console.log(`Server running on port ${PORT}`);
  console.log(`API proxy functionality has been removed from this system.`);
});
