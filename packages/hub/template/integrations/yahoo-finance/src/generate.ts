// convertSchema.js
import { zodToJsonSchema } from "zod-to-json-schema";
import Schemas from "./schemas";
import fs from "node:fs/promises";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const schemasDir = path.join(__dirname, "..", "files", "schemas");

const generate = async () => {
  // Create schemas directory if it doesn't exist
  if (!existsSync(schemasDir)) {
    await fs.mkdir(schemasDir, { recursive: true });
  } else {
    // Clear existing schema files
    const files = await fs.readdir(schemasDir);
    await Promise.all(
      files.map((file) => fs.unlink(path.join(schemasDir, file)))
    );
  }

  // Generate new schema files
  await Promise.all(
    Schemas.map(async ({ name, schema }) => {
      const jsonSchema = zodToJsonSchema(schema, name);
      const formatted = JSON.stringify(jsonSchema, null, 2);
      console.log(formatted);
      await fs.writeFile(path.join(schemasDir, `${name}.json`), formatted);
    })
  );
};

generate().catch(console.error);
