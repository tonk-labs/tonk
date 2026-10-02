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

test("the in-space account ceremony wears the FABB's bright twin in both schemes", () => {
  const block = appStyles.match(/#tonk-register\[data-fabb-task\]\s*\{([^}]*)\}/)?.[1] ?? "";
  assert.match(
    block,
    /color-scheme:\s*light;/,
    "the ceremony stands in for the FABB, which never follows the page's dark swap",
  );
  for (const [name, source] of [
    ["--ink", "--fabb-ink"],
    ["--on-ink", "--fabb-on-ink"],
    ["--soft", "--fabb-ink-soft"],
    ["--ring", "--fabb-ring"],
    ["--wash", "--fabb-hover"],
    ["--modal", "--fabb-panel"],
  ]) {
    assert.ok(
      block.includes(`${name}: var(${source});`),
      `${name} must be pinned to ${source}, or the dark theme repaints the ceremony dark`,
    );
  }
  assert.doesNotMatch(
    block,
    /--dim:/,
    "the backdrop dims the page in the page's own scheme",
  );
});
