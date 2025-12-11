# Oracle #2: Investigation of @tonk/core/slim Module Resolution

## Initial Hypotheses

At the start of investigation, I formed several hypotheses:

1. **Published npm package missing exports**: The npm package `@tonk/core@0.1.2` might not include the `/slim` subpath export
2. **TypeScript moduleResolution mismatch**: The `moduleResolution: "bundler"` setting might not support package.json exports properly
3. **Lockfile inconsistency**: The lockfile might be locking to the wrong version
4. **Workspace interference**: The monorepo workspace configuration might be overriding the specific version requirement
5. **CI environment difference**: Something specific to the CI environment differs from local

## Research Path

### Step 1: Prior Context Recovery

I leveraged the TotalRecall synthesis graph and discovered highly relevant prior investigations:
- PR #333 that added launcher/desktonk to the monorepo
- Previous investigations into desktonk CI failures with similar `@tonk/core` resolution issues
- The decision to use published npm versions (0.1.1, later 0.1.2) instead of workspace links

Key prior learnings:
- `host-web` had a similar issue with `workspace:*` causing CI failures
- The pattern of using npm versions for launcher/desktonk was explicitly established
- `linkWorkspacePackages = false` was added to bunfig.toml to prevent workspace linking

### Step 2: Package.json Analysis

Examined `packages/core-js/package.json`:
- Version: `0.1.1` (local, but npm has `0.1.2`)
- Exports field correctly defines:
  ```json
  "./slim": {
    "types": "./dist/index-slim.d.ts",
    "import": "./dist/index-slim.js",
    "default": "./dist/index-slim.js"
  }
  ```

Examined `packages/launcher/package.json`:
- `"@tonk/core": "0.1.2"` - specific npm version, NOT workspace:*
- This was intentionally set to avoid workspace linking

### Step 3: npm Package Verification

Verified npm package `@tonk/core@0.1.2` is correctly published:
```bash
npm view @tonk/core@0.1.2 exports --json
```
Result confirmed the exports field is present and correct in the published package.

### Step 4: Local Installation Verification

After fresh `bun install` in launcher directory:
- `node_modules/@tonk/core/package.json` shows version `0.1.2`
- `node_modules/@tonk/core/dist/index-slim.d.ts` exists (3.2k)
- `node_modules/@tonk/core/dist/index-slim.js` exists (3.2k)
- TypeScript resolves `@tonk/core/slim` correctly

**Local works correctly - no `@tonk/core/slim` resolution error**

### Step 5: CI Failure Analysis

Retrieved actual CI failure logs from GitHub Actions run `20105982275`:

```
src/launcher/services/bundleManager.ts(1,31): error TS2307: Cannot find module '@tonk/core/slim' or its corresponding type declarations.
```

**Critical discovery in CI bun install output:**
```
+ @tonk/core@workspace:packages/core-js
```

**CI is installing from workspace instead of npm!**

### Step 6: Root Cause Investigation

Examined `bun.lock` (committed to repo):
```
"@tonk/core": ["@tonk/core@workspace:packages/core-js"],
...
"@tonk/launcher/@tonk/core": ["@tonk/core@0.1.2", "", {}, "sha512-..."],
```

The lockfile has TWO resolutions:
1. Global: `@tonk/core` -> `workspace:packages/core-js`
2. Specific: `@tonk/launcher/@tonk/core` -> `0.1.2` from npm

**But CI uses the global workspace resolution instead of the launcher-specific one.**

### Step 7: bunfig.toml Analysis

`packages/launcher/bunfig.toml`:
```toml
[install]
linkWorkspacePackages = false
```

This SHOULD prevent workspace linking, but CI shows it's being ignored.

### Step 8: CI Workflow Analysis

The workflow runs:
1. `find . -name bunfig.toml -exec sed -i '/\[install.security\]/,/^$/d' {} \;` - strips security scanner
2. `bun install` from `packages/launcher` directory
3. Build/test

The sed command was verified to NOT remove `linkWorkspacePackages = false`.

### Step 9: gitignore Investigation

`packages/core-js/.gitignore` includes `dist/`

This means in CI:
- `packages/core-js/dist/` does NOT exist
- When Bun resolves to workspace package, TypeScript can't find the types

## Dead Ends

### Dead End 1: TypeScript Configuration
Initially suspected `moduleResolution: "bundler"` might not support exports. But:
- `resolvePackageJsonExports: true` is set
- Local TypeScript compiles fine with `@tonk/core/slim`

### Dead End 2: Version Mismatch
Thought maybe npm didn't have 0.1.2 yet. But:
- `npm view` confirmed 0.1.2 exists
- Published 2025-12-10T11:56:41.353Z (same day as investigation)

### Dead End 3: CI Container Differences
Thought maybe Docker container had different behavior. But:
- Local bun 1.3.3 vs CI bun 1.3.4 - minor difference
- Both should support `linkWorkspacePackages`

## Key Discoveries

### Discovery 1: Bun Workspace Resolution Bug

When running `bun install` from a workspace subdirectory:
- Bun detects the parent workspace from `package.json` with `workspaces: ["packages/*"]`
- Despite `linkWorkspacePackages = false` and specific version `"0.1.2"`
- Bun still resolves to the workspace package

The lockfile has both resolutions, but CI ignores the per-package override.

### Discovery 2: CI Creates New Lockfile

CI logs show:
```
Resolving dependencies
...
Saved lockfile
```

When running from subdirectory, Bun regenerates the lockfile with different resolutions than the committed root lockfile.

### Discovery 3: Missing dist/ Directory Chain

The failure chain:
1. CI runs `bun install` from `packages/launcher`
2. Bun resolves `@tonk/core` to `workspace:packages/core-js`
3. `packages/core-js/dist/` is gitignored, doesn't exist in CI
4. TypeScript can't find `./dist/index-slim.d.ts`
5. Build fails with "Cannot find module '@tonk/core/slim'"

### Discovery 4: Prior Investigations Predicted This

The TotalRecall synthesis graph showed previous investigations identified the same root cause pattern for desktonk's `@tonk/core` issues. The fix of using npm version instead of workspace:* was applied, but the underlying bun workspace resolution behavior wasn't fully addressed.

## Synthesis: Answer to Core Question

### Why CI Fails While Local Works

**Root Cause**: Bun's workspace resolution behavior in CI differs from local when running `bun install` from a subdirectory.

- **Locally**: Fresh install correctly uses npm `@tonk/core@0.1.2` because the lockfile override is respected
- **In CI**: Bun regenerates the lockfile and uses global workspace resolution `workspace:packages/core-js`

The `linkWorkspacePackages = false` setting is NOT being respected when:
1. Running from a workspace subdirectory
2. The root workspace defines the package
3. A lockfile already exists with workspace resolution

### Does npm Package Include /slim Export?

**Yes, confirmed.** `@tonk/core@0.1.2` has the correct exports field with `./slim` pointing to valid files.

## Proposed Fixes

### Fix Option 1: Run bun install at Root (Recommended)

Modify `.github/workflows/launcher.yml`:
```yaml
- run: bun install  # At root, not subdirectory
- run: bun run build
  working-directory: packages/launcher
```

**Pros**: Uses the committed lockfile correctly, respects per-package resolutions
**Cons**: Installs all workspace packages (slower)

### Fix Option 2: Delete Root Lockfile Before Subdirectory Install

```yaml
- run: rm -f bun.lock
- run: bun install
  working-directory: packages/launcher
```

**Pros**: Forces fresh resolution without workspace interference
**Cons**: Non-deterministic builds, might break other things

### Fix Option 3: Extract Launcher from Workspace

Remove `packages/launcher` from workspace definition in root `package.json`:
```json
"workspaces": [
  "packages/core-js",
  "packages/relay-js",
  // Remove launcher and desktonk
]
```

**Pros**: Clean separation, no workspace interference
**Cons**: Breaks development workflows, needs separate lockfile

### Fix Option 4: Use npm: Protocol (Experimental)

```json
"@tonk/core": "npm:@tonk/core@0.1.2"
```

**Pros**: Explicitly forces npm registry
**Cons**: Non-standard, may have other side effects

### Recommended Solution

**Fix Option 1** is the cleanest. The CI workflow should:
1. Run `bun install` at root level (uses committed lockfile correctly)
2. Run build/test commands with `working-directory: packages/launcher`

This aligns with how monorepos are typically managed and ensures the per-package lockfile resolutions are respected.

## Confidence & Caveats

### High Confidence

- The npm package `@tonk/core@0.1.2` is correctly published with `/slim` export
- The committed `bun.lock` has correct per-package resolution
- CI is getting workspace resolution instead of npm
- The `dist/` directory being gitignored causes the type resolution failure

### Medium Confidence

- `linkWorkspacePackages = false` not being respected is likely a Bun bug/limitation
- Running `bun install` at root would fix the issue

### Low Confidence

- Whether this is a known Bun issue or undocumented behavior
- Whether there's a better configuration approach

## Divergent Possibilities

### Alternative Interpretation 1: Intentional Bun Behavior

Perhaps Bun intentionally prefers workspace packages when running from a subdirectory, even with `linkWorkspacePackages = false`. The flag might only affect root-level installs.

### Alternative Interpretation 2: Stale Lockfile

The lockfile might be out of sync with package.json changes. Running `bun install` at root and committing the updated lockfile might resolve the issue.

### Alternative Interpretation 3: CI Caching Issues

Though not evidenced, CI caching of node_modules or lockfiles could cause stale resolutions. A clean cache might help.

## Evidence Summary

| Evidence | Finding |
|----------|---------|
| npm package | `/slim` export exists and is correct |
| Local install | Works correctly, gets `@tonk/core@0.1.2` |
| CI install | Gets `workspace:packages/core-js` |
| CI logs | Shows "Saved lockfile" indicating regeneration |
| bunfig.toml | Has `linkWorkspacePackages = false` |
| bun.lock | Has both global and per-package resolutions |
| core-js/dist | Gitignored, doesn't exist in CI |

## Sources

- [Bun Workspaces Documentation](https://bun.com/docs/pm/workspaces)
- [Bun linkWorkspacePackages - v1.2.16 Release](https://bun.sh/blog/bun-v1.2.16)
- [Bun Issue #10889: Workspace dependency resolution](https://github.com/oven-sh/bun/issues/10889)
- [Setting up Changesets with Bun Workspaces](https://ianm.com/posts/2025-08-18-setting-up-changesets-with-bun-workspaces)
