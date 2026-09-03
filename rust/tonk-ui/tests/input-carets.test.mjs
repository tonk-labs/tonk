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

test("the settings display name has visible input affordance", () => {
  const rule = appStyles.match(/ui-account-settings \.sname \{([\s\S]*?)\}/);
  assert.ok(rule, "the display-name input must have an authored style rule");
  assert.match(
    rule[1],
    /border-bottom:\s*1px solid var\(--ink\)/,
    "the editable value must keep the design system's visible field underline",
  );
  assert.match(
    rule[1],
    /flex:\s*0 1 18ch/,
    "the underline must read as a bounded input rather than a row divider",
  );
});

test("active account fields use a measured two-line row spanning its full width", () => {
  assert.match(
    registration,
    /class="orow mblk editing" id="tonk-register-email-row"/,
  );
  assert.match(registration, /class_list\(\)\.remove_1\("editing"\)/);
  assert.match(
    appStyles,
    /\.tonk-ceremony \.orow\.editing \{[\s\S]*?box-sizing: border-box;[\s\S]*?height: 60px;[\s\S]*?grid-template-rows: 13px 20px;[\s\S]*?gap: 7px;/,
  );
  assert.match(
    appStyles,
    /\.tonk-ceremony \.orow\.editing \.ed \{[\s\S]*?display: block;[\s\S]*?inline-size: 100%;[\s\S]*?max-inline-size: none;/,
    "the password manager must see the field's real trailing edge",
  );
  assert.doesNotMatch(
    appStyles,
    /\.tonk-ceremony \.ed\[autocomplete~="webauthn"\]/,
    "input padding shifts the password-manager affordance away from the row edge",
  );
});

test("an anchored account ceremony cannot scroll away from its hub bar", () => {
  assert.match(
    appStyles,
    /html:has\(#tonk-register\[data-anchored\]\[open\]:not\(\[data-suspended\]\)\),[\s\S]*?body:has\(#tonk-register\[data-anchored\]\[open\]:not\(\[data-suspended\]\)\)\s*\{\s*overflow: hidden;/,
    "the top page must be locked while its fixed account page is open",
  );
  assert.match(
    appStyles,
    /#tonk-register\[data-anchored\]\s*\{[\s\S]*?overflow: hidden;[\s\S]*?overscroll-behavior: none;/,
    "the account page must neither scroll nor chain wheel input to the page behind it",
  );
});
