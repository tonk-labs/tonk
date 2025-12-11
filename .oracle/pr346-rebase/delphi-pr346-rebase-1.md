# Oracle #1 Investigation: PR #346 Rebase Strategy

## Initial Hypotheses

When I started this investigation, I had several initial hypotheses:

1. **Biome migration would cause widespread formatting conflicts** - The removal of prettier in favor of biome (#347) would reformat many files, causing merge conflicts even in files with no semantic changes.

2. **The API change to `updateFile` would be the most critical conflict** - The `updateFile` API change in #348 would require semantic adaptation of the PR's code.

3. **Lockfiles and build artifacts would be trivial conflicts** - `bun.lock` and `service-worker-bundled.js` conflicts would be noise, resolvable by regeneration.

4. **The PR's actual feature code is isolated** - The Dock component and thumbnail improvements are mostly additive, not conflicting.

---

## Research Path

### Avenue 1: Branch Topology Analysis

**Actions taken:**
- `git log --oneline main..HEAD` - Verified 4 commits on PR branch
- `git log --oneline HEAD..main` - Identified 5 commits on main since merge-base
- `git merge-base main HEAD` - Confirmed merge-base at 927ed02

**Findings:**
- PR Branch commits (oldest to newest):
  1. `1239520` - Dark mode styling, delete textEditor.module.css  
  2. `129e44a` - Thumbnail generation, add Dock component
  3. `f533eab` - Add Dock to RootLayout, fix dark mode button
  4. `099ba31` - Add tinki icon image

- Main branch commits since merge-base:
  1. `e789f42` - React CVE security patch
  2. `6d36ac3` - `updateDoc` auto patch/diff feature
  3. `5a387ac` - Merge CVE fix
  4. `fccb499` - **CRITICAL:** Remove prettier in favor of biome
  5. `357ca2c` - Publish core-js with new `updateFile` API

### Avenue 2: File Overlap Analysis

**Actions taken:**
- `git diff --name-only 927ed02..HEAD` - 24 files changed on PR branch
- `git diff --name-only 927ed02..main` - 300+ files changed on main (mostly biome formatting)
- Cross-referenced to find 12 files modified in both branches

**Conflicting files identified:**
1. `bun.lock`
2. `packages/desktonk/app.tonk` (binary)
3. `packages/desktonk/src/components/layout/RootLayout.tsx`
4. `packages/desktonk/src/components/ui/button/button.tsx`
5. `packages/desktonk/src/features/desktop/components/Desktop.tsx`
6. `packages/desktonk/src/features/editor/components/editor.css`
7. `packages/desktonk/src/features/editor/components/tiptap-ui-primitive/line-numbers/LineNumbers.css`
8. `packages/desktonk/src/features/text-editor/TextEditorApp.tsx`
9. `packages/desktonk/src/features/text-editor/hooks/useEditorVFSSave.ts`
10. `packages/desktonk/src/features/text-editor/textEditor.module.css`
11. `packages/launcher/package.json`
12. `packages/launcher/public/app/service-worker-bundled.js`

### Avenue 3: Actual Rebase Test

**Actions taken:**
- Created test branch `test-rebase-oracle1`
- Ran `git rebase main` to observe actual conflicts
- Analyzed conflict markers and auto-merges

**First commit (1239520) conflicts:**
```
CONFLICT (content): bun.lock
CONFLICT (content): packages/desktonk/app.tonk (binary)
CONFLICT (content): packages/desktonk/src/features/text-editor/TextEditorApp.tsx
CONFLICT (modify/delete): packages/desktonk/src/features/text-editor/textEditor.module.css
CONFLICT (content): packages/launcher/package.json
CONFLICT (content): packages/launcher/public/app/service-worker-bundled.js
```

**Successfully auto-merged:**
- `editor.css`
- `LineNumbers.css`

### Avenue 4: Semantic Analysis of Key Conflicts

**Actions taken:**
- Compared diffs side-by-side for each conflicting file
- Analyzed the nature of changes (formatting vs semantic)

**Critical findings by file:**

#### TextEditorApp.tsx - COMPLEX SEMANTIC + FORMATTING
- **Main changes:** Import reordering (biome alphabetization), arrow function syntax
- **PR changes:** Added `useMemo`, removed CSS module import, refactored with `useMemo` for content rendering, inline styles with dark mode support
- **Conflict type:** Import section conflict + later body changes
- **Resolution strategy:** Use PR's imports but add to biome-style ordering. Keep ALL PR semantic changes.

#### useEditorVFSSave.ts - CRITICAL SEMANTIC CONFLICT
- **Main changes:** Changed `vfs.writeFile()` to `vfs.updateFile()` (API migration)
- **PR changes:** Added 75-line `generateThumbnail()` function, added logic to preserve `desktopMeta` during saves, moved thumbnail generation to unmount
- **The issue:** Main's `updateFile` is designed for partial updates (auto-merge), while PR used `writeFile` with manual metadata preservation
- **Resolution strategy:** Keep PR's `generateThumbnail` function but adapt `saveToVFS` to use `updateFile` instead of `writeFile`. The `updateFile` API should handle the metadata preservation automatically.

#### RootLayout.tsx - SIMPLE CONFLICT
- **Main changes:** Import reordering only (biome)
- **PR changes:** Added `Dock` import and `<Dock />` component in JSX
- **Resolution:** Reorder imports biome-style AND add the Dock import/component

#### button.tsx - SIMPLE CONFLICT  
- **Main changes:** Import reordering, long line wrapping (biome formatting)
- **PR changes:** Added `dark:text-white` to default variant
- **Resolution:** Take main's formatting and add PR's dark mode class

#### Desktop.tsx - MEDIUM COMPLEXITY
- **Main changes:** Arrow function syntax changes (`error =>` instead of `(error) =>`), multiline formatting
- **PR changes:** Added `thumbnailVersion` prop checking, props comparison logic in shape updates (lines 96-143)
- **Resolution:** Apply main's formatting style to PR's semantic additions

#### textEditor.module.css - DELETE/MODIFY CONFLICT
- **Main changes:** Whitespace formatting only
- **PR changes:** FILE DELETED (styles moved inline to TextEditorApp.tsx)
- **Resolution:** Delete the file (accept PR's deletion)

#### launcher/package.json - VERSION CONFLICT
- **Main:** `@tonk/core: "^0.1.3"`, `react: "^19.1.2"`
- **PR:** `@tonk/core: "0.1.2"`
- **Resolution:** Take main's versions (more recent)

#### editor.css - AUTO-MERGED SUCCESSFULLY
- **Main:** Indentation changes, quote style fix
- **PR:** Added dark mode rules at end of file
- **Auto-merge worked** because changes were in different sections

#### LineNumbers.css - AUTO-MERGED SUCCESSFULLY
- **Main:** Multiline font-family formatting
- **PR:** Changed dark mode color value
- **Auto-merge worked** because changes were on different lines

---

## Dead Ends

### Dead End 1: Trying dry-run rebase
Git doesn't support `--dry-run` for rebase. Had to create a test branch to actually see conflicts.

### Dead End 2: Initial diff output confusion
The `git diff --stat 927ed02..HEAD` output showed 331 files because I was comparing against the current working directory state, not pure commit comparison. The actual PR only touches ~24 files across 4 commits.

### Dead End 3: Expecting more RootLayout conflicts
Initially thought RootLayout would have complex conflicts due to layout changes. In reality, main only reordered imports - a trivial conflict.

---

## Key Discoveries

### Discovery 1: Biome Migration Impact is Surgical
The biome migration touched 200+ files with formatting changes, but for the PR's scope, only the import ordering and arrow function syntax create conflicts. Most CSS and structural code auto-merges cleanly.

### Discovery 2: The `updateFile` API Change is Semantic
Main introduced `vfs.updateFile()` as a simpler API for partial updates in commit `357ca2c`. The PR's `useEditorVFSSave` hook explicitly uses `vfs.writeFile()` with manual metadata preservation. This needs adaptation, not just conflict resolution.

### Discovery 3: PR's CSS Module Deletion is Intentional
The PR deliberately deleted `textEditor.module.css` and moved styles inline to `TextEditorApp.tsx`. The modify/delete conflict should resolve to deletion.

### Discovery 4: Binary File Conflicts are Unavoidable
`app.tonk` is a binary bundle that changed on both branches. This must be resolved by rebuilding after rebase.

### Discovery 5: Build Artifacts Should Be Regenerated
`service-worker-bundled.js` is a build artifact. Conflicts here are noise - it will be rebuilt by the build process.

---

## Synthesis: Recommended Rebase Strategy

### Pre-Rebase Steps

1. **Ensure clean working directory**
   ```bash
   git stash -u  # if needed
   git checkout ramram/feat/tinki-dock
   ```

2. **Create backup branch**
   ```bash
   git branch ramram/feat/tinki-dock-backup
   ```

### Rebase Execution

```bash
git rebase main
```

### Conflict Resolution Order (by commit)

#### Commit 1 (1239520): Dark mode styling
**Conflicts to resolve:**

1. **bun.lock** - Accept main's version, then run `bun install` after rebase
   ```bash
   git checkout --theirs bun.lock
   git add bun.lock
   ```

2. **app.tonk** - Accept main's version (will rebuild later)
   ```bash
   git checkout --theirs packages/desktonk/app.tonk
   git add packages/desktonk/app.tonk
   ```

3. **TextEditorApp.tsx** - Manual resolution:
   - Use biome-ordered imports from main
   - Add `useMemo` to the imports
   - Keep ALL PR semantic changes (useMemo refactor, inline styles)
   ```bash
   # Edit file to combine changes
   git add packages/desktonk/src/features/text-editor/TextEditorApp.tsx
   ```

4. **textEditor.module.css** - Delete (PR's intent)
   ```bash
   git rm packages/desktonk/src/features/text-editor/textEditor.module.css
   ```

5. **launcher/package.json** - Take main's versions
   ```bash
   git checkout --theirs packages/launcher/package.json
   git add packages/launcher/package.json
   ```

6. **service-worker-bundled.js** - Take main's version (rebuilds)
   ```bash
   git checkout --theirs packages/launcher/public/app/service-worker-bundled.js
   git add packages/launcher/public/app/service-worker-bundled.js
   ```

7. Continue rebase:
   ```bash
   git rebase --continue
   ```

#### Commit 2 (129e44a): Thumbnail generation
**Potential conflicts:**
- `useEditorVFSSave.ts` may conflict if main's formatting touched this file
- `Desktop.tsx` may need format reconciliation

**Key adaptation needed:**
- In `useEditorVFSSave.ts`, change `vfs.writeFile()` calls to `vfs.updateFile()` where appropriate
- The `generateThumbnail` function can likely use `updateFile` for the metadata update

#### Commit 3 (f533eab): Add Dock to layout
**Expected conflicts:**
- `RootLayout.tsx` - Add Dock import in biome-sorted order
- `button.tsx` - Apply dark mode class with main's formatting

#### Commit 4 (099ba31): Tinki icon
- No conflicts expected (new file only)

### Post-Rebase Steps

1. **Regenerate lockfile:**
   ```bash
   bun install
   ```

2. **Run biome format on changed files:**
   ```bash
   bun run format
   ```

3. **Rebuild service worker:**
   ```bash
   cd packages/launcher && bun run build:sw:dev
   ```

4. **Rebuild app.tonk bundle:**
   ```bash
   cd packages/desktonk && bun run build
   ```

5. **Test the application:**
   ```bash
   bun run dev
   ```

6. **Force push the rebased branch:**
   ```bash
   git push --force-with-lease origin ramram/feat/tinki-dock
   ```

---

## Confidence & Caveats

### High Confidence
- Import ordering conflicts are trivial and well-understood
- textEditor.module.css should be deleted
- Lockfile and build artifacts should be regenerated
- Most CSS conflicts auto-merge successfully

### Medium Confidence  
- The `updateFile` API adaptation should work, but needs testing
- Desktop.tsx thumbnail logic should integrate cleanly with formatting changes
- The 4 commits can be rebased sequentially without squashing

### Low Confidence / Caveats
- The `app.tonk` binary file may have meaningful changes from both sides - after accepting main's version, verify the app still has PR's features
- Service worker behavior after rebuild - test thoroughly
- Possible transitive dependency issues from version bumps

---

## Divergent Possibilities

### Alternative 1: Squash Before Rebase
Squash the 4 PR commits into 1-2 commits, then rebase. This reduces conflict resolution rounds but loses granular history.

**Pros:** Fewer conflict resolution iterations
**Cons:** Harder to bisect issues later, loses attribution clarity

### Alternative 2: Merge Instead of Rebase  
Use `git merge main` instead of rebase, preserving branch topology.

**Pros:** Preserves full history, simpler conflict resolution (one round)
**Cons:** Messier history, harder to review changes linearly

### Alternative 3: Rebase with --strategy-option
Use `git rebase main -X theirs` or `-X ours` for specific files to auto-resolve formatting conflicts.

**Pros:** Faster for formatting-only conflicts
**Cons:** May accidentally lose semantic changes if applied too broadly

### Recommendation
**Standard rebase (Avenue 1)** is recommended because:
1. The PR has only 4 focused commits
2. Conflicts are predictable and manageable
3. Clean linear history aids review
4. The semantic conflicts (useEditorVFSSave) need manual attention anyway

---

## Summary

The rebase of PR #346 onto main is **feasible with manual intervention**. The key challenges are:

1. **TextEditorApp.tsx** - Must reconcile biome formatting with useMemo refactor
2. **useEditorVFSSave.ts** - Must adapt to `updateFile` API (semantic change)
3. **textEditor.module.css** - Must confirm deletion is accepted
4. **Build artifacts** - Must regenerate after rebase

Estimated time: 20-30 minutes for experienced developer familiar with the codebase.

Risk level: **Medium** - The semantic API change requires careful attention, but the changes are well-scoped.
