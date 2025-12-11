# Oracle #2 Deep Investigation: PR #346 Rebase Strategy

## Initial Hypotheses

When I started this investigation, I formed the following hypotheses:

1. **Hypothesis 1**: The biome migration (#347) would cause widespread formatting conflicts that are cosmetic, not semantic
2. **Hypothesis 2**: The core feature changes (Dock component) would be isolated and safe to rebase
3. **Hypothesis 3**: Lock files (bun.lock) and generated files (service-worker-bundled.js) would be the hardest to resolve
4. **Hypothesis 4**: A simple git rebase main would work with manual conflict resolution

## Research Path

### Avenue 1: Commit Analysis

I analyzed the commit structure on both sides:

**PR Branch (4 commits from merge-base 927ed02):**
1. 1239520 - feat(desktonk): improve dark mode styling and refactor text editor CSS
2. 129e44a - fix(desktonk): improve thumbnail generation and display  
3. f533eab - feat(desktonk): add Dock to layout and fix dark mode styling
4. 099ba31 - feat(tinki): icon (binary asset only)

**Main Branch (5 commits since merge-base):**
1. e789f42 - fix(security): update React to patch CVE-2025-55182
2. 6d36ac3 - feat(core): updateDoc auto patch/diff (#344)
3. 5a387ac - Merge CVE fix
4. fccb499 - chore: TON-1642: remove prettier in favor of biome (#347) **[CRITICAL]**
5. 357ca2c - chore(core-js): publish with new updateFile (#348) **[API CHANGE]**

**Key Discovery**: The order matters! Commit 1239520 (first on PR branch) conflicts immediately because it modifies files that biome later reformats on main.

### Avenue 2: File-by-File Conflict Analysis

I performed a live rebase simulation and identified actual conflicts:

| File | Conflict Type | Severity | Resolution Strategy |
|------|---------------|----------|---------------------|
| bun.lock | Content | LOW | Accept main, re-run bun install |
| packages/desktonk/app.tonk | Binary | MEDIUM | Accept PR version (has new data) |
| packages/desktonk/src/features/text-editor/TextEditorApp.tsx | Content | HIGH | Manual merge - import order + semantic changes |
| packages/desktonk/src/features/text-editor/textEditor.module.css | Delete/Modify | HIGH | Accept deletion (PR removes this file) |
| packages/launcher/package.json | Content | MEDIUM | Take main @tonk/core: ^0.1.3 and react: ^19.1.2 |
| packages/launcher/public/app/service-worker-bundled.js | Content | LOW | Regenerate after rebase |

**Files that auto-merged successfully but need verification:**
- packages/desktonk/src/features/editor/components/editor.css
- packages/desktonk/src/features/editor/components/tiptap-ui-primitive/line-numbers/LineNumbers.css

### Avenue 3: Semantic Conflict Analysis (CRITICAL FINDING)

**The updateFile vs writeFile API Change:**

Main branch commit 357ca2c changed useEditorVFSSave.ts from:
```typescript
await vfs.writeFile(filePath, { content: { text } });
```
to:
```typescript
await vfs.updateFile(filePath, { text });
```

BUT the PR branch has a completely refactored version that still uses writeFile with a more complex structure:
```typescript
await vfs.writeFile(filePath, {
  content: {
    text,
    ...(existingDesktopMeta && { desktopMeta: existingDesktopMeta }),
  },
});
```

**This is a SEMANTIC conflict that wont show as a git conflict** because the PR rewrote the entire function. The PRs approach preserves desktopMeta during saves, which is important for thumbnail management.

**Recommended Resolution**: Keep the PRs logic but update to use updateFile API:
```typescript
await vfs.updateFile(filePath, {
  text,
  ...(existingDesktopMeta && { desktopMeta: existingDesktopMeta }),
});
```

### Avenue 4: Biome Configuration Analysis

Main now has:
- Root biome.json with base configuration
- packages/desktonk/biome.json extending root with package-specific settings

Key biome settings affecting this rebase:
- arrowParentheses: asNeeded (root) vs always (desktonk)
- lineWidth: 80 (root) vs 100 (desktonk)
- Import sorting is now enforced by biome

**Important**: After rebase, running bun run format will auto-fix most style issues.

## Dead Ends

1. **Tried**: Looking for a way to auto-merge CSS files - biome does not handle CSS formatting, so these conflicts need manual attention
2. **Tried**: Checking if the binary app.tonk could be regenerated - it is an Automerge bundle that accumulates state, both versions are valid but divergent
3. **Tried**: Looking for pre-commit hooks that might auto-format - found husky config but biome check happens in CI, not pre-commit

## Key Discoveries

### Discovery 1: First Commit Causes Most Conflicts
Commit 1239520 (dark mode styling) touches the most conflicting files. Consider reordering commits to apply cleaner ones first.

### Discovery 2: The textEditor.module.css Deletion
The PR deletes textEditor.module.css and inlines the styles in TextEditorApp.tsx. Main only reformatted this file. Git shows this as deleted by them and modified in HEAD - **the correct resolution is to accept the deletion**.

### Discovery 3: New Files Are Clean
The new dock feature files (Dock.tsx, useDockActions.ts, index.ts) and tinki-icon.png have no conflicts. They will apply cleanly after resolving the conflicting commits.

### Discovery 4: Service Worker is Generated
service-worker-bundled.js is a build artifact. Do not try to manually resolve - just regenerate it after rebase.

## Synthesis: Recommended Rebase Strategy

### Option A: Standard Interactive Rebase (RECOMMENDED)

```bash
# Step 1: Start interactive rebase
git checkout ramram/feat/tinki-dock
git rebase -i main

# Step 2: Reorder commits to minimize conflicts
# In editor, change order to:
# pick 099ba31 feat(tinki): icon  # Binary only, no conflicts
# pick f533eab feat(desktonk): add Dock to layout and fix dark mode styling
# pick 129e44a fix(desktonk): improve thumbnail generation and display
# pick 1239520 feat(desktonk): improve dark mode styling and refactor text editor CSS

# Step 3: Resolve conflicts for each commit (see detailed resolution below)

# Step 4: After successful rebase, run formatting
bun run format

# Step 5: Regenerate build artifacts
cd packages/launcher && bun run build:sw:dev

# Step 6: Update dependencies
cd ../.. && bun install

# Step 7: Verify
bun run lint
bun run build
```

### Detailed Conflict Resolutions

#### For TextEditorApp.tsx:
1. Accept mains import ordering (biome style)
2. Add useMemo to the imports from react
3. Keep PRs logic changes (inline styles, useMemo for content)
4. Remove the styles import (file is deleted)

```typescript
// Final imports should look like:
import type { JSONContent } from '@tiptap/react';
import { useEffect, useMemo, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { Button } from '@/components/ui/button/button';
// ... rest in alphabetical order per biome
```

#### For useEditorVFSSave.ts:
1. Keep PRs full refactor (thumbnail generation on unmount)
2. **IMPORTANT**: Change writeFile calls to updateFile where appropriate
3. The large generateThumbnail function stays as-is (uses writeFile for new files, which is correct)

#### For launcher/package.json:
```json
{
  "@tonk/core": "^0.1.3",
  "react": "^19.1.2",
  "react-dom": "^19.1.2"
}
```

#### For bun.lock:
Accept mains version, then run bun install to regenerate.

#### For app.tonk:
Accept PRs version (contains new canvas data).

#### For textEditor.module.css:
```bash
git rm packages/desktonk/src/features/text-editor/textEditor.module.css
```

### Option B: Merge Instead of Rebase

If rebase proves too complex:

```bash
git checkout ramram/feat/tinki-dock
git merge main
# Resolve conflicts same as above
git commit -m "Merge main into feat/tinki-dock"
```

**Pros**: Preserves original commit history
**Cons**: Creates merge commit, messier history

### Option C: Squash and Rebase

If commit history is not important:

```bash
# Squash all PR commits into one
git checkout ramram/feat/tinki-dock
git reset --soft 927ed02
git commit -m "feat(desktonk): add Dock component with Tinki app launcher

- Add macOS-style Dock component to RootLayout
- Implement useDockActions hook for file creation
- Improve thumbnail generation (generate on unmount)
- Fix dark mode styling across text editor
- Add Tinki icon asset"

git rebase main
# Resolve conflicts once instead of per-commit
```

**Pros**: Single conflict resolution pass, clean history
**Cons**: Loses granular commit history

## Confidence Assessment

### High Confidence
- Conflict list is accurate (verified via live rebase)
- textEditor.module.css should be deleted
- bun.lock should be regenerated via bun install
- service-worker-bundled.js should be regenerated
- New dock feature files will apply cleanly

### Medium Confidence  
- Commit reordering will reduce conflicts (theoretical, not tested)
- The updateFile vs writeFile API is semantically compatible
- app.tonk PR version is the correct choice

### Low Confidence
- Exact time to complete rebase (estimated 15-30 minutes)
- Whether all tests will pass after rebase (need to run full test suite)

## Caveats

1. **Test the updateFile API change**: The PRs saveToVFS function may need adjustment to work with the new API signature
2. **Binary file resolution**: app.tonk contains Automerge CRDT data - both versions are valid but contain different document states
3. **CI may catch additional issues**: Biome lint rules may flag issues not visible during local rebase

## Divergent Possibilities

### Alternative 1: Pre-format PR branch before rebase
```bash
# On PR branch, apply biome formatting first
git checkout ramram/feat/tinki-dock
bun run format
git commit -m "chore: apply biome formatting"
git rebase main
```
This could reduce formatting conflicts but may create other issues.

### Alternative 2: Cherry-pick specific commits
If only the Dock feature is needed:
```bash
git checkout main
git cherry-pick 099ba31  # icon
git cherry-pick <new-dock-files-only>  # extract just the new files
```
Loses the dark mode and thumbnail fixes.

### Alternative 3: Feature branch reset
```bash
# Start fresh from main with the desired changes
git checkout main
git checkout -b ramram/feat/tinki-dock-v2
# Manually copy over the new feature files
# Manually apply the logic changes
```
Most work but cleanest result.

## Recommended Action

**Use Option A (Standard Interactive Rebase) with the commit reorder.**

The conflicts are manageable, and preserving the commit history provides value for understanding the features evolution. The key is to:

1. Reorder commits to apply cleanest ones first
2. Be careful about the updateFile API change  
3. Run bun run format after rebase to ensure biome compliance
4. Regenerate lock files and build artifacts
5. Run full test suite before pushing

**Estimated Time**: 20-30 minutes for an experienced developer familiar with the codebase.

---

*Oracle #2 Investigation Complete*
*Confidence: HIGH for conflict identification, MEDIUM for resolution strategies*
