import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const appStyles = readFileSync(join(HERE, "..", "styles.css"), "utf8");

test("the shell headline cover does not style authored headings", () => {
  assert.doesNotMatch(
    appStyles,
    /(?:^|\})\s*h1\s*,\s*h2\s*,\s*h3\s*,\s*h4\s*,\s*h5\s*,\s*h6\s*\{/m,
    "a bare heading selector is injected into every guest and styles authored views",
  );
  assert.match(
    appStyles,
    /\.tonk-shell-heading\s*,\s*\.space-banner\s+:where\(h1,\s*h2,\s*h3,\s*h4,\s*h5,\s*h6\)\s*\{/,
    "the headline cover must remain available only to explicit shell chrome",
  );
});
