# Delphi Synthesis: core-slim-resolution

## Executive Summary

Three oracles independently investigated why `@tonk/core/slim` module resolution fails in the launcher package. All three reached a **unanimous conclusion**: the published npm package `@tonk/core@0.1.2` correctly includes the `/slim` subpath export with all necessary files (`dist/index-slim.js`, `dist/index-slim.d.ts`). The module resolution problem is NOT caused by a missing export in the npm package.

The root cause involves the interaction between Bun's workspace resolution and the monorepo configuration. When `bun install` runs, despite `linkWorkspacePackages = false` being set in bunfig.toml, certain conditions can cause Bun to resolve `@tonk/core` to the local workspace package (`packages/core-js`) instead of fetching from npm. Since `dist/` is gitignored, the workspace package lacks the compiled output files in CI, causing TypeScript to fail with "Cannot find module '@tonk/core/slim'".

The current branch (`ramram/feat/tinki-dock`) appears to have the correct fix: using an explicit npm version `"@tonk/core": "0.1.2"` instead of `workspace:*`. However, on `main` branch, PR #344 introduced `"@tonk/core": "workspace:*"` which creates a fragile configuration that works by accident due to `linkWorkspacePackages = false` falling back to npm.

## Convergent Findings

All three oracles independently confirmed the following with **high confidence**:

### 1. npm Package is Correctly Published
- `@tonk/core@0.1.2` has proper `exports` field with `./slim` subpath
- All necessary files exist: `dist/index-slim.js` (3.2KB), `dist/index-slim.d.ts` (3.2KB)
- This has been true for ALL versions (0.1.0, 0.1.1, 0.1.2)

### 2. TypeScript Configuration is Correct
- `moduleResolution: "bundler"` fully supports package.json exports
- `tsc --traceResolution` confirms successful resolution when npm package is installed
- The resolution works locally with clean installs

### 3. bunfig.toml Configuration Exists
- All oracles found `linkWorkspacePackages = false` in `packages/launcher/bunfig.toml`
- This setting is intended to prevent workspace linking

### 4. dist/ Directory is Gitignored
- `packages/core-js/.gitignore` excludes `dist/`
- In CI, the workspace package lacks compiled output
- If Bun resolves to workspace instead of npm, TypeScript fails

### 5. Local Builds Work
- All oracles verified local builds succeed after clean install
- The npm package installs correctly as a real directory (not symlink)

## Divergent Findings

The oracles diverged on several key interpretations:

### Divergence 1: Current CI Status

| Oracle | Assessment |
|--------|------------|
| Oracle #1 | CI currently passes; the configuration "works accidentally" |
| Oracle #2 | CI is failing; logs show workspace resolution occurring |
| Oracle #3 | CI has been passing recently; error may not exist anymore |

**Analysis**: Oracle #2 cited specific CI run `20105982275` showing workspace resolution, while Oracle #1 triggered a fresh run that passed, and Oracle #3 found recent successful runs. This suggests an **intermittent or environment-dependent issue** - the problem is real but not consistently reproducible.

### Divergence 2: Root Cause Interpretation

| Oracle | Root Cause |
|--------|------------|
| Oracle #1 | Configuration inconsistency: `workspace:*` in package.json conflicts with `linkWorkspacePackages = false` |
| Oracle #2 | Bun bug/limitation: workspace resolution overrides `linkWorkspacePackages = false` when running from subdirectory |
| Oracle #3 | Problem may be historical/already fixed in current codebase |

**Analysis**: These are complementary perspectives. Oracle #1 focused on the declarative configuration conflict, Oracle #2 investigated the mechanical Bun behavior, and Oracle #3 questioned whether the problem still exists. The truth likely combines all three: there was a configuration problem, Bun's behavior makes it fragile, and the current branch has already fixed it.

### Divergence 3: Recommended Fix

| Oracle | Primary Recommendation |
|--------|------------------------|
| Oracle #1 | Change `workspace:*` to `0.1.2` in launcher's package.json |
| Oracle #2 | Run `bun install` at root level in CI workflow |
| Oracle #3 | Verify if issue still exists; clear caches if needed |

**Analysis**: These are not mutually exclusive. Oracle #1's fix is the cleanest (matches documented intent), Oracle #2's fix addresses the underlying Bun behavior, and Oracle #3's fix is diagnostic.

## Unique Discoveries

### Oracle #1: PR #344 Archaeology
- Discovered that commit `6d36ac3` from PR #344 (merged Dec 10, 2025) changed launcher from `"@tonk/core": "0.1.1"` to `"@tonk/core": "workspace:*"`
- This violated the established pattern where launcher/desktonk use specific npm versions
- The current branch already has the fix (`"0.1.2"`)

### Oracle #2: Lockfile Dual Resolution
- Found the lockfile contains TWO different resolutions:
  - Global: `@tonk/core` -> `workspace:packages/core-js`
  - Per-package: `@tonk/launcher/@tonk/core` -> `0.1.2` from npm
- CI regenerates lockfile and uses global resolution
- CI logs show "Saved lockfile" indicating regeneration

### Oracle #3: TS2345 vs TS2307 Distinction
- Found the ONLY current TypeScript error is `TS2345` (type mismatch), NOT `TS2307` (module resolution)
- This suggests the module resolution issue may already be fixed in the current state
- Provided multiple alternative solutions (paths mapping, separate package, vendoring)

## Composite Answer

### Why is `@tonk/core/slim` not found when building launcher?

The failure occurs through this chain:

1. **Configuration conflict**: PR #344 changed `@tonk/core` from a specific npm version to `workspace:*` in launcher's package.json
2. **Bun install behavior**: When running from a subdirectory, Bun may use the root workspace resolution even with `linkWorkspacePackages = false`
3. **Missing dist/**: The workspace package (`packages/core-js/dist/`) is gitignored and doesn't exist in CI
4. **TypeScript failure**: TypeScript cannot find `./dist/index-slim.d.ts` in the workspace package

**The npm package is fine.** The problem is Bun resolving to the workspace package instead of fetching from npm.

### How should we fix it?

**Primary fix (already in current branch):**
```json
"@tonk/core": "0.1.2"
```

This explicit version:
- Matches the documented pattern for launcher/desktonk
- Eliminates reliance on `linkWorkspacePackages` workaround
- Provides deterministic, reproducible builds
- Works in all environments (local, CI, extraction)

**Secondary fix (defense in depth):**
```yaml
# In .github/workflows/launcher.yml
- run: bun install  # At root level
- run: bun run build
  working-directory: packages/launcher
```

Running `bun install` at root ensures the committed lockfile is used correctly with per-package resolution.

## Confidence Assessment

### High Confidence
- npm package `@tonk/core@0.1.2` is correctly structured with `/slim` export
- TypeScript's `moduleResolution: "bundler"` supports package exports
- `dist/` being gitignored is a factor when workspace resolution occurs
- The current branch has `"@tonk/core": "0.1.2"` which is the correct fix
- `main` branch has `workspace:*` which is problematic

### Medium Confidence
- `linkWorkspacePackages = false` may not be fully respected in subdirectory installs
- The issue is intermittent/environment-dependent rather than consistent
- Running `bun install` at root would fix CI behavior

### Low Confidence / Uncertain
- Why PR #344 changed to `workspace:*` (intent unknown)
- Whether this is a documented Bun limitation or a bug
- Exact conditions that trigger workspace vs npm resolution
- Whether other packages in the monorepo have similar issues

## Recommended Actions

### Immediate (High Priority)
1. **Merge current branch fix**: The `ramram/feat/tinki-dock` branch already has `"@tonk/core": "0.1.2"` - ensure this gets to main
2. **Audit main branch**: Apply same fix to main if the branch doesn't merge soon
   ```bash
   git checkout main
   # Edit packages/launcher/package.json: "@tonk/core": "0.1.2"
   git commit -m "fix(launcher): use npm @tonk/core version instead of workspace"
   ```

### Short-term (Medium Priority)
3. **Check desktonk**: Verify `packages/desktonk/package.json` uses npm version, not `workspace:*`
4. **Add documentation**: Comment in package.json or CLAUDE.md explaining why launcher/desktonk must use npm versions
5. **Update lockfile**: Run `bun install` at root and commit updated bun.lock

### Long-term (Low Priority)
6. **CI workflow improvement**: Consider running `bun install` at root level for more reliable resolution
7. **Version automation**: Create a script or workflow to update npm version when @tonk/core is published
8. **Monitor Bun issues**: Track if `linkWorkspacePackages` subdirectory behavior is a known issue

## Appendix: Oracle Contributions

### Oracle #1 Contributions
- **Primary focus**: Configuration archaeology and intent analysis
- **Key discovery**: PR #344 introduced the `workspace:*` regression
- **Methodology**: Git blame/log to trace configuration changes
- **Unique insight**: The configuration "works accidentally" due to npm fallback
- **Recommended approach**: Direct package.json fix to explicit version

### Oracle #2 Contributions
- **Primary focus**: Bun behavior analysis and CI log investigation
- **Key discovery**: Lockfile has dual resolution; CI regenerates and uses global
- **Methodology**: CI log analysis, bun behavior testing
- **Unique insight**: `linkWorkspacePackages = false` may not work from subdirectories
- **Recommended approach**: CI workflow change to run install at root

### Oracle #3 Contributions
- **Primary focus**: Verification and reproduction attempt
- **Key discovery**: Could not reproduce error; only TS2345 errors exist currently
- **Methodology**: Clean install simulation, TypeScript trace resolution
- **Unique insight**: Problem may already be fixed or intermittent
- **Recommended approach**: Verify issue exists before applying fixes; provided alternative solutions

### Synthesis Value
By combining all three investigations:
- We understand the historical cause (Oracle #1)
- We understand the mechanical behavior (Oracle #2)
- We understand the current state (Oracle #3)
- We have high confidence in both the diagnosis and the fix
