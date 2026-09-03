import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const UI = join(HERE, "..");
const WORKSPACE = join(UI, "..", "tonk-workspace", "src");

const accountStyles = readFileSync(join(UI, "src", "account.css"), "utf8");
const appStyles = readFileSync(join(UI, "styles.css"), "utf8");
const registration = readFileSync(
  join(UI, "src", "register_dialog.rs"),
  "utf8",
);
const settings = readFileSync(
  join(WORKSPACE, "ui_account_settings.html"),
  "utf8",
);

test("authored text fields keep the browser's native insertion caret", () => {
  for (const [name, source] of [
    ["account styles", accountStyles],
    ["application styles", appStyles],
  ]) {
    assert.doesNotMatch(source, /caret-shape\s*:\s*block/i, `${name} forces a block caret`);
    assert.doesNotMatch(
      source,
      /caret-color\s*:\s*transparent/i,
      `${name} hides the native caret`,
    );
  }

  assert.doesNotMatch(
    registration,
    /<i class="cur"/,
    "registration fields must not overlay a terminal cursor",
  );
  assert.doesNotMatch(
    settings,
    /<i class="cur"/,
    "settings fields must not overlay a terminal cursor",
  );
});

test("account deletion names its confirmation phrase beside a native input", () => {
  assert.match(
    settings,
    /type <b data-delete-confirm-label>delete account<\/b> to confirm:/,
  );
  assert.match(settings, /<input class="armfield"[^>]+data-delete-confirm/);
  assert.doesNotMatch(settings, /contenteditable/);
});
