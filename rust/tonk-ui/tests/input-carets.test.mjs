import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const UI = join(HERE, "..");
const LIBRARY = join(UI, "..", "tonk-core", "assets", "library", "profile.yaml");

const accountStyles = readFileSync(join(UI, "src", "account.css"), "utf8");
const appStyles = readFileSync(join(UI, "styles.css"), "utf8");
const registration = readFileSync(
  join(UI, "src", "register_dialog.rs"),
  "utf8",
);
// The settings panel is markup on the profile branch: the
// `<account-settings>` element's contents, plus the registration view's
// `settings` facet, which carries the display name and the deletion
// dialog.
const library = readFileSync(LIBRARY, "utf8");
const panel = library.split("<account-settings>\n")[1]?.split("</account-settings>")[0];
const registered = library
  .split("    settings: |\n      <div data-account-registered>")[1]
  ?.split("\n\n")[0];
assert.ok(panel, "the settings panel markup must be in the profile library");
assert.ok(registered, "the registration settings facet must be in the profile library");
const settings = `${registered}\n${panel}`;

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
  const rule = appStyles.match(/account-settings \.sname \{([\s\S]*?)\}/);
  assert.ok(rule, "the display-name input must have an authored style rule");
  assert.match(
    rule[1],
    /background:\s*var\(--sep\)/,
    "the editable value must sit on the design system's visible field surface",
  );
  assert.match(
    rule[1],
    /flex:\s*0 1 28ch/,
    "the surface must read as a bounded input rather than a row divider",
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
    /#tonk-register\[data-anchored\]\s*\{[^}]*overflow: hidden;[^}]*overscroll-behavior: none;/,
    "the account page must neither scroll nor chain wheel input to the page behind it",
  );
  assert.match(
    appStyles,
    /#tonk-register\[data-anchored\] \.ocol\s*\{[^}]*overflow: auto;[^}]*overscroll-behavior: none;/,
    "the content panel must remain scrollable without chaining to the page behind it",
  );
});

test("an anchored account ceremony obscures the hub account menu beneath it", () => {
  assert.match(
    appStyles,
    /#tonk-register\[data-anchored\] \.ocol\s*\{[\s\S]*?background:\s*var\(--page\);/,
    "the opaque column backing must cover the gaps between ceremony rows",
  );
});
