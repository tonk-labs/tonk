# Oracle #1: @tonk/core/slim Module Resolution Investigation

## Core Question
Why is `@tonk/core/slim` not found when building the launcher package, and how should we fix it given the constraint that desktonk and launcher should NOT link to workspace packages?

## Initial Hypotheses

When I started this investigation, I had several hypotheses:

1. **H1: Published npm package missing /slim export** - The published `@tonk/core` package might not include the `/slim` subpath export or the corresponding files
2. **H2: TypeScript moduleResolution configuration** - The launcher's tsconfig might use a moduleResolution mode that doesn't support package exports
3. **H3: Workspace linking interference** - Despite `linkWorkspacePackages = false`, workspace linking might still be occurring
4. **H4: Version mismatch** - The version specified in package.json might not have the /slim export
5. **H5: CI vs local environment difference** - Something in the CI environment differs from local builds

## Research Path

### Phase 1: Verify Published Package Contents

**Action**: Checked npm registry for `@tonk/core@0.1.2`:

```bash
npm view @tonk/core@0.1.2 exports --json
npm pack @tonk/core@0.1.2 --pack-destination /tmp
```

**Finding**: The published package at version 0.1.2 **DOES contain** the `/slim` export:
- `dist/index-slim.d.ts` (3.2KB)
- `dist/index-slim.d.ts.map` (729B)
- `dist/index-slim.js` (3.2KB)

The exports field is correctly configured:
```json
{
  "./slim": {
    "types": "./dist/index-slim.d.ts",
    "import": "./dist/index-slim.js",
    "default": "./dist/index-slim.js"
  }
}
```

**Conclusion**: H1 is FALSE - the published package is complete.

### Phase 2: Verify TypeScript Configuration

**Action**: Checked `packages/launcher/tsconfig.json`:

```json
{
  "compilerOptions": {
    "moduleResolution": "bundler",
    ...
  }
}
```

**Finding**: `moduleResolution: "bundler"` fully supports package.json exports. Running `tsc --traceResolution` confirmed successful resolution:

```
Module name '@tonk/core/slim' was successfully resolved to
'.../node_modules/@tonk/core/dist/index-slim.d.ts' with Package ID '@tonk/core/dist/index-slim.d.ts@0.1.2'
```

**Conclusion**: H2 is FALSE - TypeScript configuration is correct.

### Phase 3: Investigate workspace:* vs Specific Version

**CRITICAL DISCOVERY**: The current `main` branch has a conflict in configuration:

**packages/launcher/package.json (on main):**
```json
"@tonk/core": "workspace:*"
```

**packages/launcher/bunfig.toml:**
```toml
[install]
linkWorkspacePackages = false
```

**Current branch (ramram/feat/tinki-dock) has:**
```json
"@tonk/core": "0.1.2"
```

This means:
- Someone changed launcher from `"@tonk/core": "0.1.1"` to `"@tonk/core": "workspace:*"` on main
- The current branch changed it to `"@tonk/core": "0.1.2"` (specific npm version)

### Phase 4: Trace the workspace:* Introduction

**Action**: Git archaeology to find when `workspace:*` was introduced:

```bash
git log --oneline -p -- packages/launcher/package.json | grep "0.1.2"
git show 6d36ac3 -- packages/launcher/package.json
```

**Finding**: Commit `6d36ac3` (feat(core): TON-1638: `updateDoc` auto patch/diff) changed:
```diff
-    "@tonk/core": "0.1.1",
+    "@tonk/core": "workspace:*",
```

This was part of PR #344 merged today (Dec 10, 2025).

### Phase 5: Understand CI Behavior with workspace:* + linkWorkspacePackages=false

**IMPORTANT DISCOVERY**: When `linkWorkspacePackages = false` is set in bunfig.toml, Bun will:
1. NOT link to the local workspace package
2. INSTEAD fall back to installing from npm registry
3. Use whatever version is in the lockfile or latest available

**Evidence from CI run 20106459073:**
```
+ @tonk/core@0.1.1 (v0.1.2 available)
```

CI installed `@tonk/core@0.1.1` from npm even though package.json says `workspace:*`.

### Phase 6: Test Standalone Environment

**Action**: Simulated CI by extracting launcher to `/tmp` without parent workspace:

```bash
rm -rf /tmp/launcher-ci-test
cp -r packages/launcher/* /tmp/launcher-ci-test/
cd /tmp/launcher-ci-test && bun install
```

**Finding**: When `linkWorkspacePackages = false` is set, the install succeeded and fetched from npm.

When `workspace:*` is used WITHOUT `linkWorkspacePackages = false`, Bun fails:
```
error: Workspace dependency "@tonk/core" not found
Searched in "./*"
```

## Dead Ends

### Dead End 1: CI Currently Failing
I initially assumed CI was failing now. After triggering a fresh CI run on main, it passed. The `linkWorkspacePackages = false` setting prevents the workspace:* from breaking CI by falling back to npm.

### Dead End 2: Missing dist/ in CI
I hypothesized that workspace linking would point to local `packages/core-js` which lacks `dist/` (gitignored). While this WOULD be an issue if workspace linking occurred, `linkWorkspacePackages = false` prevents it.

### Dead End 3: Version Resolution Confusion
The lockfile shows `@tonk/core@0.1.1` but CI installs 0.1.1 even with `workspace:*`. This is expected behavior with `linkWorkspacePackages = false`.

## Key Discoveries

### Discovery 1: The Configuration Works (Accidentally)
The combination of:
- `"@tonk/core": "workspace:*"` in package.json
- `linkWorkspacePackages = false` in bunfig.toml

Actually works because Bun falls back to npm. However, this is fragile and inconsistent.

### Discovery 2: Version Pinning is Lost
With `workspace:*` + `linkWorkspacePackages = false`, the installed version depends on:
1. What's in bun.lock (if present)
2. Latest version from npm (if not in lock)

CI is installing 0.1.1 even though 0.1.2 is available because of lockfile.

### Discovery 3: PR #344 Introduced Inconsistency
The change to `workspace:*` in PR #344 violated the established pattern where launcher/desktonk use specific npm versions, not workspace references. The comment in bunfig.toml explicitly says "avoid linking" but the package.json now says "use workspace".

### Discovery 4: Current Branch Has Correct Fix
The current branch (`ramram/feat/tinki-dock`) has the correct configuration:
```json
"@tonk/core": "0.1.2"
```

This is explicit, deterministic, and matches the intended pattern.

## Synthesis: Answer to Core Question

### Why might `@tonk/core/slim` not be found?

The issue would occur in these scenarios:

1. **If linkWorkspacePackages was not set to false** - Bun would try to link to `packages/core-js` which has no `dist/` directory in CI (gitignored), causing "Cannot find module" errors.

2. **If running outside CI** - Local builds might see a different version of @tonk/core depending on whether the workspace link exists.

3. **If the lockfile gets corrupted or deleted** - Bun might install a different version than expected.

### Why CI is Currently Passing

CI passes because:
1. `linkWorkspacePackages = false` makes Bun ignore the `workspace:*` and fetch from npm
2. The lockfile pins to version 0.1.1 which has the `/slim` export
3. The npm package is properly published with all necessary files

### The Correct Fix

**Change launcher's package.json from:**
```json
"@tonk/core": "workspace:*"
```

**To:**
```json
"@tonk/core": "0.1.2"
```

This fix:
1. Explicitly declares the dependency version
2. Matches the documented pattern for launcher/desktonk
3. Doesn't rely on `linkWorkspacePackages = false` as a workaround
4. Will consistently install from npm in all environments
5. Is what the current branch (`ramram/feat/tinki-dock`) already has

## Confidence & Caveats

### High Confidence
- Published @tonk/core packages (0.1.0, 0.1.1, 0.1.2) all contain /slim export
- TypeScript resolves @tonk/core/slim correctly when installed from npm
- The `linkWorkspacePackages = false` setting prevents workspace linking
- The current branch has the correct fix

### Medium Confidence
- The original error report may have been from a transient state before lockfile was committed
- The behavior of `workspace:*` + `linkWorkspacePackages = false` is intentional in Bun

### Low Confidence / Uncertain
- Why PR #344 changed to `workspace:*` when the established pattern was npm versions
- Whether there are other packages with similar inconsistencies

## Divergent Possibilities

### Alternative 1: Keep workspace:* with Better Documentation
If the intent IS to use workspace versions during development, keep `workspace:*` but document that:
- `linkWorkspacePackages = false` MUST be set
- Lockfile must be committed
- Version will float based on what's in npm

### Alternative 2: Use Conditional Dependencies
Some build tools support environment-based dependency resolution. This could allow workspace linking in development but npm in CI.

### Alternative 3: Publish from CI
Instead of relying on published npm packages, CI could build @tonk/core first, then build launcher. This would make workspace linking viable.

## Recommended Action

**Immediate fix for main branch:**
```bash
git checkout main
# Edit packages/launcher/package.json
# Change "@tonk/core": "workspace:*" to "@tonk/core": "0.1.2"
git commit -m "fix(launcher): use npm @tonk/core version instead of workspace"
```

**Also check desktonk:**
```bash
git show main:packages/desktonk/package.json | grep "@tonk/core"
```

If desktonk also has `workspace:*`, apply same fix.

## Files Examined

- `/Users/alexander/Node/tonk/packages/core-js/package.json` - Defines @tonk/core exports
- `/Users/alexander/Node/tonk/packages/launcher/package.json` - Depends on @tonk/core
- `/Users/alexander/Node/tonk/packages/launcher/tsconfig.json` - TypeScript configuration
- `/Users/alexander/Node/tonk/packages/launcher/bunfig.toml` - Bun install settings
- `/Users/alexander/Node/tonk/packages/core-js/.gitignore` - Shows dist/ is gitignored
- `/Users/alexander/Node/tonk/.github/workflows/launcher.yml` - CI workflow
- `/Users/alexander/Node/tonk/bun.lock` - Root lockfile with resolution data

## Sources Referenced

- [Bun bunfig.toml Documentation](https://bun.com/docs/runtime/bunfig)
- [Bun Workspaces Guide](https://bun.com/docs/guides/install/workspaces)
- [TypeScript Module Resolution](https://www.typescriptlang.org/docs/handbook/module-resolution.html)
- [npm package exports guide](https://hirok.io/posts/package-json-exports)
- npm registry: @tonk/core@0.1.2 package contents
