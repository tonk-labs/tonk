# Delphi Oracle #3: @tonk/core/slim Module Resolution Investigation

## Core Question
Why is `@tonk/core/slim` not found when building the launcher package, and how should we fix it given the constraint that desktonk and launcher should NOT link to workspace packages?

---

## 1. Initial Hypotheses

When I started this investigation, I formed several hypotheses:

1. **H1: npm package missing /slim export** - The published `@tonk/core@0.1.2` might not have the `/slim` subpath export configured
2. **H2: npm package missing dist files** - The `index-slim.js` and `index-slim.d.ts` files might not be included in the published package
3. **H3: Workspace resolution override** - Despite `linkWorkspacePackages = false`, bun might be resolving to the workspace package which lacks `dist/` in CI
4. **H4: TypeScript moduleResolution issue** - The tsconfig might use a moduleResolution mode that doesn't support package.json exports
5. **H5: CI environment difference** - Something specific to the CI environment (container, bun version) causes different behavior
6. **H6: Cached stale package** - CI might be using a cached older version of `@tonk/core` that predates the `/slim` export

---

## 2. Research Path

### Step 1: Verify npm package exports

First, I checked if the npm package has the `/slim` export:

```bash
npm view @tonk/core@0.1.2 exports --json
```

**Result:**
```json
{
  ".": {
    "types": "./dist/index.d.ts",
    "import": "./dist/index.js",
    "default": "./dist/index.js"
  },
  "./slim": {
    "types": "./dist/index-slim.d.ts",
    "import": "./dist/index-slim.js",
    "default": "./dist/index-slim.js"
  },
  "./wasm": "./dist/tonk_core_bg.wasm"
}
```

**Conclusion: H1 DISPROVED** - The npm package DOES have the `/slim` export properly configured.

### Step 2: Verify npm package contains dist files

```bash
npm pack @tonk/core@0.1.2 --dry-run 2>&1 | grep -i slim
```

**Result:**
```
npm notice 3.2kB dist/index-slim.d.ts
npm notice 729B dist/index-slim.d.ts.map
npm notice 3.2kB dist/index-slim.js
```

**Conclusion: H2 DISPROVED** - The npm package includes all `/slim` files.

### Step 3: Check all versions for /slim export

```bash
npm view @tonk/core@0.1.0 exports --json
npm view @tonk/core@0.1.1 exports --json
```

**Result:** ALL three versions (0.1.0, 0.1.1, 0.1.2) have the `/slim` export. The feature has been available since the initial release.

### Step 4: Verify bunfig.toml configuration

Checked `/Users/alexander/Node/tonk/packages/launcher/bunfig.toml`:

```toml
[install]
# avoid linking
linkWorkspacePackages = false
linker = "hoisted"
```

This configuration should prevent workspace linking. Also confirmed same config exists in:
- `/Users/alexander/Node/tonk/bunfig.toml` (root)
- `/Users/alexander/Node/tonk/packages/desktonk/bunfig.toml`

### Step 5: Test clean install from subdirectory (simulate CI)

```bash
rm -rf node_modules packages/*/node_modules
cd packages/launcher && bun install
```

**Result:** Successfully installed `@tonk/core@0.1.2` as a REAL DIRECTORY (not symlink):

```
/Users/alexander/Node/tonk/packages/launcher/node_modules/@tonk/core: directory
```

Verified package contents:
- `package.json` with version "0.1.2" and exports for `/slim`
- `dist/index-slim.d.ts` present
- `dist/index-slim.js` present

**Conclusion: H3 PARTIALLY VERIFIED** - The `linkWorkspacePackages = false` setting IS working correctly in local environment.

### Step 6: Verify TypeScript resolution

Ran TypeScript with trace resolution:

```bash
bun x tsc --noEmit --traceResolution 2>&1 | grep -A5 "@tonk/core/slim"
```

**Result:**
```
Module name '@tonk/core/slim' was successfully resolved to
'/Users/alexander/Node/tonk/packages/launcher/node_modules/@tonk/core/dist/index-slim.d.ts'
with Package ID '@tonk/core/dist/index-slim.d.ts@0.1.2'.
```

**Conclusion: H4 DISPROVED** - TypeScript with `moduleResolution: "bundler"` correctly resolves the `/slim` subpath export.

### Step 7: Check CI workflow

Read `/Users/alexander/Node/tonk/.github/workflows/launcher.yml`:

```yaml
jobs:
  build:
    runs-on: ubuntu-latest
    container: oven/bun:1
    steps:
      - run: bun install
        working-directory: packages/launcher
      - run: bun run build
        working-directory: packages/launcher
```

The CI:
- Uses `oven/bun:1` container
- Runs `bun install` from `packages/launcher` subdirectory
- Has no custom caching of node_modules

### Step 8: Check recent CI status

```bash
gh run list --workflow=launcher.yml -L 10
```

**Result:** CI has been PASSING recently on main branch:
- 2025-12-09: Multiple successful runs
- Most recent failure was 2025-12-08 but due to a different error (`Script not found "build:host"`)

### Step 9: Test TypeScript compilation after clean install

```bash
cd packages/launcher && bun x tsc --noEmit
```

**Result:**
```
src/launcher/sw/message-handlers/file-ops.ts(214,7): error TS2345: Argument of type 'unknown'
is not assignable to parameter of type 'string | number | boolean | JsonValue | null'.
```

**CRITICAL FINDING:** The ONLY TypeScript error is `TS2345` (a type mismatch), NOT a module resolution error! The `@tonk/core/slim` import resolves successfully.

---

## 3. Dead Ends

### Dead End 1: Workspace symlink resolution
I initially suspected bun might create symlinks to workspace packages despite `linkWorkspacePackages = false`. However, after clean install, the package was correctly installed from npm as a real directory.

### Dead End 2: npm package version differences
I checked if older versions lacked the `/slim` export, but all three versions (0.1.0, 0.1.1, 0.1.2) have had it since initial release.

### Dead End 3: CI-specific failure
I tried to find the actual CI logs showing the `@tonk/core/slim` error but found that:
- Recent CI runs on main are passing
- The most recent failure was due to an unrelated error

### Dead End 4: dist/package.json interference
I noticed there's a nested `package.json` in `dist/` from wasm-pack with different content:
```json
{
  "name": "tonk-core",
  "version": "0.1.0",
  "main": "tonk_core.js"
}
```
However, this doesn't interfere with module resolution since TypeScript/Node use the root `package.json`.

---

## 4. Key Discoveries

### Discovery 1: The problem may not currently exist
After thorough investigation with completely clean installs simulating CI conditions, I could NOT reproduce the `@tonk/core/slim` module resolution error. The only TypeScript error present is a type mismatch (`TS2345`) unrelated to module resolution.

### Discovery 2: npm package is correctly structured
The published `@tonk/core@0.1.2` package has:
- Correct `exports` field with `/slim` subpath
- All necessary dist files (`index-slim.js`, `index-slim.d.ts`)
- Types field ordered correctly (`types` before `default`)

### Discovery 3: Configuration is correct
- `bunfig.toml` has `linkWorkspacePackages = false`
- `tsconfig.json` has `moduleResolution: "bundler"` which supports package exports
- TypeScript successfully resolves `@tonk/core/slim` to the npm package

### Discovery 4: Historical context
From totalrecall synthesis, previous CI failures with `@tonk/core` were due to:
- `host-web` package using `workspace:*` dependency
- `dist/` directory being gitignored
- CI running from wrong directory

These issues were DIFFERENT from the `/slim` subpath issue and have been fixed.

---

## 5. Synthesis: Answer to the Core Question

### Why is `@tonk/core/slim` not found?

**Current Status: The error may not exist in the current codebase.**

After exhaustive investigation:
1. The npm package `@tonk/core@0.1.2` correctly exports `/slim`
2. Local builds successfully resolve the module
3. CI has been passing on main branch
4. Clean installs from subdirectory (simulating CI) work correctly

**If the error IS occurring, the most likely causes are:**

1. **Stale cache or lockfile** - A cached version of `@tonk/core` from before the `/slim` export was published (though all versions have it)

2. **Transitive workspace resolution** - If another package in the dependency tree uses `workspace:*` for `@tonk/core`, it could cause resolution issues in CI where `dist/` doesn't exist

3. **Concurrent session interference** - If root `bun install` runs while/before subdirectory install, workspace linking might occur

### Recommended Fix (if issue exists)

1. **Verify the actual error** - Get CI logs showing the exact error message and context

2. **Clear caches in CI**:
   ```yaml
   - run: rm -rf ~/.bun/install/cache
     if: failure()
   ```

3. **Ensure clean install order**:
   ```yaml
   - run: rm -rf node_modules
     working-directory: packages/launcher
   - run: bun install
     working-directory: packages/launcher
   ```

4. **Add explicit lockfile** - Create `packages/launcher/bun.lockb` to pin dependencies

5. **Verify no workspace interference**:
   - Ensure no package.json in parent directories accidentally triggers workspace mode
   - Add `"workspaces": []` to `packages/launcher/package.json` if needed

---

## 6. Confidence & Caveats

### High Confidence
- npm package `@tonk/core@0.1.2` has correct `/slim` export (verified directly)
- TypeScript `bundler` moduleResolution supports package exports (verified by trace)
- Local builds work correctly (verified multiple times)
- `linkWorkspacePackages = false` is configured correctly

### Medium Confidence
- CI should work the same as local (based on workflow analysis, not actual CI reproduction)
- The error may have been resolved in recent commits

### Low Confidence / Uncertain
- The exact scenario that triggers the error (couldn't reproduce)
- Whether the issue affects current codebase or was already fixed
- CI container-specific behavior that might differ from local

### Caveats
1. I could not reproduce the actual error message `Cannot find module '@tonk/core/slim'`
2. The investigation was done locally, not in actual CI environment
3. The current branch (`ramram/feat/tinki-dock`) may have different state than when error occurred

---

## 7. Divergent Possibilities

### Possibility A: The error is historical/fixed
The error may have occurred in a previous state of the codebase and has since been fixed by:
- Publishing `@tonk/core@0.1.2` with correct exports
- Fixing bunfig.toml configuration
- Updating CI workflow

### Possibility B: Environment-specific issue
The error may only occur in specific environments:
- Specific bun version in `oven/bun:1` container
- Linux vs macOS differences in path resolution
- Container filesystem behavior

### Possibility C: Race condition / timing issue
The error may be intermittent:
- Parallel bun install operations
- npm registry propagation delays
- Cache invalidation timing

### Possibility D: Different entry point
The error may occur not during `tsc` but during:
- Vite build (service worker bundling)
- Runtime execution
- Test execution

### Alternative Solution Approaches

1. **Use relative import instead of subpath export**:
   ```typescript
   // Instead of:
   import { TonkCore } from '@tonk/core/slim';
   // Use:
   import { TonkCore } from '@tonk/core/dist/index-slim.js';
   ```
   This bypasses export resolution but is less clean.

2. **Bundle @tonk/core into launcher**:
   Instead of depending on npm package, vendor the core library directly.

3. **Create separate @tonk/core-slim package**:
   Publish the slim version as a separate package without subpath export complexity.

4. **Add explicit types path in tsconfig**:
   ```json
   {
     "compilerOptions": {
       "paths": {
         "@tonk/core/slim": ["./node_modules/@tonk/core/dist/index-slim.d.ts"]
       }
     }
   }
   ```

---

## Files Examined

- `/Users/alexander/Node/tonk/packages/core-js/package.json`
- `/Users/alexander/Node/tonk/packages/launcher/package.json`
- `/Users/alexander/Node/tonk/packages/desktonk/package.json`
- `/Users/alexander/Node/tonk/packages/launcher/tsconfig.json`
- `/Users/alexander/Node/tonk/packages/launcher/tsconfig.app.json`
- `/Users/alexander/Node/tonk/packages/launcher/bunfig.toml`
- `/Users/alexander/Node/tonk/bunfig.toml`
- `/Users/alexander/Node/tonk/.github/workflows/launcher.yml`
- `/Users/alexander/Node/tonk/.github/workflows/desktonk.yml`
- `/Users/alexander/Node/tonk/packages/launcher/src/launcher/sw/wasm-init.ts`
- `/Users/alexander/Node/tonk/packages/core-js/src/index-slim.ts`
- `/Users/alexander/Node/tonk/packages/launcher/node_modules/@tonk/core/package.json`
- `/Users/alexander/Node/tonk/packages/launcher/node_modules/@tonk/core/dist/index-slim.d.ts`

---

## Session ID
`oracle-3-core-slim-resolution`

## Investigation Date
2025-12-10
