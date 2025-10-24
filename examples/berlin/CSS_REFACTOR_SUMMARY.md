# CSS Refactor Summary

## Problem Identified

The project had `.tiptap.ProseMirror` styles scattered across 10 files with **100+ occurrences**,
creating overlapping concerns and cascade conflicts. The main issues were:

1. **`paragraph-node.css`** was misnamed - it contained 214 lines of **global editor styles**, not
   paragraph-specific styles
2. **Padding conflicts** between `simple-editor.css` and `LineNumbers.css` on the same selector
3. **No clear separation** of concerns between base styles, node styles, and instance overrides
4. **Unpredictable cascade** order due to unclear import hierarchy

## Solution Implemented

### New Architecture

```
src/
├── styles/
│   ├── _variables.css (existing)
│   ├── _keyframe-animations.css (existing)
│   └── prosemirror-base.css ⭐ NEW
│
├── features/editor/
│   ├── styles/
│   │   └── editor-layout.css ⭐ NEW
│   ├── extensions/
│   │   └── LineNumbers.css ✅ REFACTORED
│   └── components/tiptap/
│       └── simple-editor.css ✅ REFACTORED
│
└── components/tiptap-node/
    └── paragraph-node/paragraph-node.css ✅ REFACTORED (214 → 19 lines)
```

### Files Created

#### 1. `/src/styles/prosemirror-base.css` (NEW)

**Purpose:** Global ProseMirror editor styles that apply to ALL instances

**Contains:**

- CSS variables for editor (colors, themes)
- Core editor behavior (caret, outline, white-space)
- Selection styles
- Text decorations (links, strikethrough, underline)
- Collaboration features (carets, labels)
- Inline features (emoji, links, mentions)
- Thread/Comment system
- Placeholder styles
- Drop cursor

**Moved from:** `paragraph-node.css` (lines 2-213)

#### 2. `/src/features/editor/styles/editor-layout.css` (NEW)

**Purpose:** Layout and positioning for the simple-editor instance

**Contains:**

- `.simple-editor-content` container layout
- Editor padding (including line numbers spacing)
- Mobile responsive padding

**Consolidated from:**

- `simple-editor.css` (layout rules)
- `LineNumbers.css` (padding conflicts)

**Resolves:** The critical padding war between simple-editor.css and LineNumbers.css

### Files Refactored

#### 3. `paragraph-node.css` (REDUCED: 214 → 19 lines)

**Before:** Global editor styles + paragraph styles **After:** Only paragraph-specific spacing

**Removed:** Everything global (moved to prosemirror-base.css)

#### 4. `simple-editor.css` (REDUCED)

**Before:** Variables + scrollbar + toolbar + font + layout **After:** Variables + scrollbar +
toolbar + font (NO layout)

**Removed:** All layout/padding rules (moved to editor-layout.css)

#### 5. `LineNumbers.css` (REDUCED)

**Before:** Line number styling + editor padding **After:** Line number styling only

**Removed:** All `.tiptap.ProseMirror` padding rules (moved to editor-layout.css)

### Import Order (CRITICAL)

Updated `/src/index.css` with documented import hierarchy:

```css
/* 1. Base variables */
@import 'tailwindcss';
@import './styles/_variables.css';

/* 2. ProseMirror base (lowest priority) */
@import './styles/prosemirror-base.css';

/* 3. Node-specific styles */
@import './components/tiptap-node/paragraph-node/paragraph-node.css';
@import './components/tiptap-node/heading-node/heading-node.css';
/* ... other nodes ... */

/* 4. Extension styles */
@import './features/editor/extensions/LineNumbers.css';

/* 5. Layout styles (higher priority) */
@import './features/editor/styles/editor-layout.css';

/* 6. Instance overrides (highest priority) */
@import './features/editor/components/tiptap/simple-editor.css';
```

## Problems Resolved

### ✅ RESOLVED: Padding War

**Before:**

- `simple-editor.css:48` set `padding: 3rem 3rem`
- `LineNumbers.css:26` set `padding-left: 4rem`
- Same selector, same specificity → unpredictable cascade

**After:**

- Single source in `editor-layout.css:28`: `padding: 3rem 3rem 3rem 4rem`

### ✅ RESOLVED: Mobile Padding Conflict

**Before:**

- `simple-editor.css:53` set `padding: 1rem 1.5rem` (shorthand)
- `LineNumbers.css:41` set `padding-left: 3rem` (longhand)
- Shorthand resets all sides → `padding-left` gets overridden

**After:**

- Single source in `editor-layout.css:39`: `padding: 1rem 1.5rem 1rem 3rem`

### ✅ RESOLVED: Misnamed Files

**Before:**

- `paragraph-node.css` contained global editor styles (misleading name)

**After:**

- `prosemirror-base.css` contains global editor styles (accurate name)
- `paragraph-node.css` only contains paragraph styles (accurate name)

### ✅ RESOLVED: Scattered Concerns

**Before:**

- 100 occurrences of `.tiptap.ProseMirror` across 10 files
- Global styles mixed with node-specific styles

**After:**

- Clear separation: base → nodes → extensions → layout → overrides
- Each file has a single, clear responsibility

## Testing Checklist

To verify the refactor works correctly:

- [ ] Build the project: `pnpm run build`
- [ ] Check for CSS import errors
- [ ] Test editor renders correctly
- [ ] Test line numbers appear correctly
- [ ] Test line numbers spacing on desktop
- [ ] Test line numbers spacing on mobile (< 480px)
- [ ] Test dark mode still works
- [ ] Test toolbar background color
- [ ] Test all node types render (headings, lists, blockquotes, code, images)
- [ ] Test selection styles
- [ ] Test placeholder text
- [ ] Compare before/after screenshots

## Migration Notes

**No breaking changes expected** - this is purely a reorganization of existing styles.

If you encounter issues:

1. Check browser console for CSS import errors
2. Verify all new files were created
3. Verify import order in `index.css`
4. Check that old content was moved correctly (not lost)

## Benefits

1. **Clear separation of concerns** - each file has one job
2. **Predictable cascade** - documented import order
3. **No more conflicts** - single source of truth for layout
4. **Better maintainability** - easier to find and modify styles
5. **Accurate naming** - files named for what they actually do
6. **Reduced duplication** - eliminated redundant selectors

## Line Count Summary

| File                         | Before | After | Change      |
| ---------------------------- | ------ | ----- | ----------- |
| paragraph-node.css           | 214    | 19    | -195 (-91%) |
| simple-editor.css            | 55     | 67    | +12 (docs)  |
| LineNumbers.css              | 43     | 42    | -1          |
| **NEW** prosemirror-base.css | 0      | 237   | +237        |
| **NEW** editor-layout.css    | 0      | 40    | +40         |

**Net result:** Better organization with clear documentation
