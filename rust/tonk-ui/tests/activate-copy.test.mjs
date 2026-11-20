// /activate in the mock's words (gooey task-page.html #activated and
// #activate-failed). The link carries no address, so a failure names "your
// email" where the mock names it.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

const page = readFileSync(new URL('../src/activate.html', import.meta.url), 'utf8');
const script = readFileSync(new URL('../src/activate.rs', import.meta.url), 'utf8');

test('an activated account says so and what it means', () => {
  assert.match(page, />account activated<\/h1>/);
  assert.match(page, /<tonk-message kind="success">Your spaces now open on your other devices\. You can close this page\.<\/tonk-message>/);
  assert.doesNotMatch(page, /account verified|You may close this tab/);
});

test('a failed activation explains how to retry', () => {
  assert.match(script, /"Tonk couldn’t confirm your email\. Check your connection and try again\."/);
  assert.equal(script.match(/show_error\(&host, NOT_CONFIRMED\)/g)?.length, 2);
  assert.doesNotMatch(script, /could not be reached|Activation failed|user_error::diagnostic/);
});
