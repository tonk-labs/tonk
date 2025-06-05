import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";
import { apiServices } from "../src/services/apiServices.ts";

// Get the directory name of the current module
const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

fs.writeFileSync(
  path.resolve(__dirname, "../dist/apiServices.json"),
  JSON.stringify(apiServices, null, 2),
);
