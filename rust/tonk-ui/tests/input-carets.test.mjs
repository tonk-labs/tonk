import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const UI = join(HERE, "..");
const LIBRARY = join(UI, "..", "tonk-core", "assets", "library", "profile.yaml");

// The activation page's styles are its view's own, in the profile library.
const accountStyles = readFileSync(LIBRARY, "utf8")
  .split("    activate: |\n")[1]
  ?.split("\n  show:")[0];
assert.ok(accountStyles, "the activation view must carry its styles");
const appStyles = readFileSync(join(UI, "styles.css"), "utf8");
// The host's sign-in panel: its markup and its own styles, on the top page.
const registration = readFileSync(join(UI, "src", "sign_in.rs"), "utf8");
const signInStyles = readFileSync(join(UI, "src", "sign_in.css"), "utf8");
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
    ["sign-in styles", signInStyles],
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
  assert.match(registration, /class="orow mblk editing"><span class="k">email address<\/span>/);
  // A settled answer is the next stage's plain row, not an editing one.
  assert.match(registration, /<div class="orow mblk"><span class="k">email<\/span><span class="v">\{email\}<\/span><\/div>/);
  assert.match(
    signInStyles,
    /\.tonk-ceremony \.orow\.editing \{[\s\S]*?display: grid;[\s\S]*?grid-template-rows: auto 48px;/,
  );
  assert.match(
    signInStyles,
    /\.tonk-ceremony \.ed \{[\s\S]*?display: block;[\s\S]*?inline-size: 100%; max-inline-size: none;/,
    "the password manager must see the field's real trailing edge",
  );
  assert.doesNotMatch(
    signInStyles,
    /\.tonk-ceremony \.ed\[autocomplete~="webauthn"\]/,
    "input padding shifts the password-manager affordance away from the row edge",
  );
});

test("the sign-in panel cannot scroll away from the page it is over", () => {
  assert.match(
    signInStyles,
    /html:has\(#tonk-register:not\(\[hidden\]\)\),[\s\S]*?body:has\(#tonk-register:not\(\[hidden\]\)\)\s*\{\s*overflow: hidden;/,
    "the top page must be locked while the panel is up",
  );
  assert.match(
    signInStyles,
    /#tonk-register \{[^}]*overflow: hidden;[^}]*overscroll-behavior: none;/,
    "the panel must neither scroll nor chain wheel input to the page behind it",
  );
  assert.match(
    signInStyles,
    /\.tonk-ceremony \.ocol \{[^}]*overflow: auto;[^}]*overscroll-behavior: none;/,
    "the content column must remain scrollable without chaining to the page behind it",
  );
  assert.match(
    signInStyles,
    /\.tonk-ceremony \.ocol \{[\s\S]*?background: var\(--card\);/,
    "the opaque column backing must cover the gaps between rows",
  );
});
