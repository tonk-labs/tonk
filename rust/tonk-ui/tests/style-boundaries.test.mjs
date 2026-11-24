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

test("the guest iframe takes no margin, so the page is never taller than the window", () => {
  assert.match(appStyles, /tonk-site > iframe \{\s*margin: 0;\s*\}/);
});

test("task pages follow the system's scheme like the hub and settings", () => {
  // gooey keeps task pages light; a light sign-in page between a dark hub
  // and dark settings is what this overrides.
  assert.match(
    appStyles,
    /:root :is\(\.tonk-task, \.fabb-page\):not\(#tonk-register\[data-fabb-task\] \*\) \{\s*color-scheme: light dark;\s*\}/,
    "task pages must follow the system, except the ceremony seated in the FABB",
  );
  const register = appStyles.match(/#tonk-register\.register-page\s*\{([^}]*)\}/)?.[1] ?? "";
  assert.doesNotMatch(register, /color-scheme:\s*light;/, "the sign-in page must not pin itself light");
});

test("a task page names tonk-look's paper for the bar's knobs, not their light fallbacks", () => {
  // gooey's rules read `var(--fabb-ink, #38182a)`: a knob left `initial`
  // falls through to the light literal, dark plum on a dark sheet.
  const page = appStyles.match(/\n\.fabb-page \{([^}]*)\}/)?.[1] ?? "";
  for (const [knob, token] of [
    ["--fabb-ink", "--tonk-text"],
    ["--fabb-ink-soft", "--tonk-text-soft"],
    ["--fabb-hover", "--tonk-wash"],
    ["--fabb-press", "--tonk-wash-strong"],
  ]) {
    assert.ok(page.includes(`${knob}: var(${token});`), `${knob} must name ${token} on a task page`);
  }
});
