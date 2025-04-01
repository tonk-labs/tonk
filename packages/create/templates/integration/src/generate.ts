// convertSchema.js
import { zodToJsonSchema } from "zod-to-json-schema";
import Schemas from "./schemas";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const generate = () => {
  Schemas.forEach(({ name, schema }) => {
    const jsonSchema = zodToJsonSchema(schema, name);
    const formatted = JSON.stringify(jsonSchema, null, 2);
    console.log(formatted);
    fs.writeFile(
      path.join(__dirname, "..", "files", `${name}.json`),
      formatted,
      (err) => {
        if (err) console.error(err);
      }
    );
  });
};

generate();
