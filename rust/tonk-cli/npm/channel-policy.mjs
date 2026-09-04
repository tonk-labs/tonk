#!/usr/bin/env node

import { pathToFileURL } from "node:url";

const inputNames = ["version", "checkoutSha", "versionTagSha", "stableSha"];

/**
 * Select the one npm channel the checked-out release is allowed to update.
 *
 * GitHub refs and events only select the checkout. Publication authority
 * comes from the release facts supplied here: the checkout must be its own
 * immutable version tag, and finals must also be the exact stable commit.
 */
export function resolveNpmChannel(input) {
  for (const name of inputNames) {
    if (typeof input?.[name] !== "string" || input[name].length === 0) {
      throw new Error(`${name} must be a non-empty string`);
    }
  }

  const { version, checkoutSha, versionTagSha, stableSha } = input;
  const versionTag = `v${version}`;

  if (checkoutSha !== versionTagSha) {
    throw new Error(
      `${versionTag} points at ${versionTagSha}, but the checkout is ${checkoutSha}; publish the tagged commit`,
    );
  }

  if (version.includes("-")) {
    if (checkoutSha === stableSha) {
      throw new Error(
        `stable cannot publish prerelease ${version}; promote a final release instead`,
      );
    }
    return "next";
  }

  if (checkoutSha !== stableSha) {
    throw new Error(
      `final ${version} must be promoted to stable before publication (checkout ${checkoutSha}, stable ${stableSha})`,
    );
  }

  return "latest";
}

function main(args) {
  if (args.length !== inputNames.length) {
    throw new Error(
      "usage: channel-policy.mjs <version> <checkout-sha> <version-tag-sha> <stable-sha>",
    );
  }

  return resolveNpmChannel(Object.fromEntries(inputNames.map((name, i) => [name, args[i]])));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    process.stdout.write(`${main(process.argv.slice(2))}\n`);
  } catch (error) {
    process.stderr.write(`channel policy error: ${error.message}\n`);
    process.exitCode = 1;
  }
}
