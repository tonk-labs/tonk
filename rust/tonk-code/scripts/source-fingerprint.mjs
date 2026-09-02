import { createHash } from "node:crypto";
import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";

export const SOURCE_FINGERPRINT_PREFIX = "tonk-code-source-sha256:";

function sourceFiles(root) {
  const files = [
    join(root, "package.json"),
    join(root, "package-lock.json"),
    join(root, "tsconfig.json"),
    join(root, "scripts", "build.mjs"),
    join(root, "scripts", "source-fingerprint.mjs"),
  ];
  const visit = (directory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) {
        if (entry.name !== "generated") visit(path);
      } else if (entry.isFile() && entry.name.endsWith(".ts")) {
        files.push(path);
      }
    }
  };
  visit(join(root, "src-js"));
  return files.sort((left, right) => {
    const leftRelative = relative(root, left);
    const rightRelative = relative(root, right);
    return leftRelative < rightRelative ? -1 : leftRelative > rightRelative ? 1 : 0;
  });
}

/** Deterministic identity of every checked-in input to the production bundle. */
export function sourceFingerprint(root) {
  const hash = createHash("sha256");
  for (const path of sourceFiles(root)) {
    hash.update(relative(root, path));
    hash.update("\0");
    hash.update(readFileSync(path));
    hash.update("\0");
  }
  return hash.digest("hex");
}
