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

test("account deletion asks in place and answers with a passkey, not a typed phrase", () => {
  assert.match(settings, /<p>Delete your account\? This can’t be undone\.<\/p>/);
  assert.match(settings, /data-ask-keep>keep account<\/button>/);
  assert.match(settings, /<span data-delete-submit-label>delete with passkey<\/span>/);
  assert.doesNotMatch(settings, /data-delete-confirm/);
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

test("the account task labels its full-width native passkey field", () => {
  const markup = readFileSync(join(UI, "src", "register.html"), "utf8");
  const forms = readFileSync(join(dirname(LIBRARY), "gooey/in-browser/fabb-forms.css"), "utf8");
  assert.match(markup, /<label class="fabb-field editing"[^>]*>\s*<span class="k">email address<\/span>\s*<input[^>]*autocomplete="username webauthn"[^>]*>\s*<\/label>/);
  assert.match(forms, /\.fabb-field :is\(input,select,textarea\)\s*\{[^}]*width:100%[^}]*caret-color:currentColor/);
  assert.doesNotMatch(markup, /class="cur"/);
});
