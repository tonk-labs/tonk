import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { resolveNpmChannel } from "./channel-policy.mjs";

const policyPath = fileURLToPath(new URL("./channel-policy.mjs", import.meta.url));

test("it_selects_next_for_a_tagged_prerelease_not_held_by_stable", () => {
  assert.equal(
    resolveNpmChannel({
      version: "0.6.12-rc.1",
      checkoutSha: "bbb",
      versionTagSha: "bbb",
      stableSha: "aaa",
    }),
    "next",
  );
});

test("it_selects_latest_for_a_tagged_final_held_by_stable", () => {
  assert.equal(
    resolveNpmChannel({
      version: "0.6.12",
      checkoutSha: "bbb",
      versionTagSha: "bbb",
      stableSha: "bbb",
    }),
    "latest",
  );
});

test("it_rejects_an_untagged_or_wrongly_tagged_commit", () => {
  assert.throws(
    () =>
      resolveNpmChannel({
        version: "0.6.12",
        checkoutSha: "bbb",
        versionTagSha: "ccc",
        stableSha: "bbb",
      }),
    (error) => {
      assert.match(error.message, /v0\.6\.12/);
      assert.match(error.message, /bbb/);
      assert.match(error.message, /ccc/);
      return true;
    },
  );
});

test("it_rejects_a_final_before_stable_holds_it", () => {
  assert.throws(
    () =>
      resolveNpmChannel({
        version: "0.6.12",
        checkoutSha: "bbb",
        versionTagSha: "bbb",
        stableSha: "aaa",
      }),
    /final 0\.6\.12 must be promoted to stable before publication/,
  );
});

test("it_rejects_a_prerelease_held_by_stable", () => {
  assert.throws(
    () =>
      resolveNpmChannel({
        version: "0.6.12-rc.1",
        checkoutSha: "bbb",
        versionTagSha: "bbb",
        stableSha: "bbb",
      }),
    /stable cannot publish prerelease 0\.6\.12-rc\.1/,
  );
});

for (let count = 0; count < 4; count += 1) {
  test(`it_rejects_${count}_cli_arguments`, () => {
    const result = spawnSync(process.execPath, [
      policyPath,
      ...["0.6.12", "bbb", "bbb", "bbb"].slice(0, count),
    ]);

    assert.notEqual(result.status, 0);
    assert.equal(result.stdout.toString(), "");
    assert.match(result.stderr.toString(), /usage: channel-policy\.mjs/);
  });
}

test("it_rejects_an_empty_cli_argument", () => {
  const result = spawnSync(process.execPath, [
    policyPath,
    "0.6.12",
    "bbb",
    "",
    "bbb",
  ]);

  assert.notEqual(result.status, 0);
  assert.equal(result.stdout.toString(), "");
  assert.match(result.stderr.toString(), /versionTagSha must be a non-empty string/);
});

test("it_prints_only_next_on_stdout", () => {
  const result = spawnSync(process.execPath, [
    policyPath,
    "0.6.12-rc.1",
    "bbb",
    "bbb",
    "aaa",
  ]);

  assert.equal(result.status, 0, result.stderr.toString());
  assert.equal(result.stdout.toString(), "next\n");
  assert.equal(result.stderr.toString(), "");
});

test("it_prints_only_latest_on_stdout", () => {
  const result = spawnSync(process.execPath, [
    policyPath,
    "0.6.12",
    "bbb",
    "bbb",
    "bbb",
  ]);

  assert.equal(result.status, 0, result.stderr.toString());
  assert.equal(result.stdout.toString(), "latest\n");
  assert.equal(result.stderr.toString(), "");
});
