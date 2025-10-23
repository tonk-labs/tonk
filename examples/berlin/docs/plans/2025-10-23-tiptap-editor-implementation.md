# TipTap Collaborative Editor Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan
> task-by-task.

**Goal:** Integrate TipTap simple-editor template with Tonk's collaborative sync middleware to
create a real-time collaborative rich text editor demo.

**Architecture:** Use TipTap's React integration with a Zustand store (following the pixelStore
pattern) wrapped in Tonk's sync middleware. Bidirectional sync: TipTap onUpdate → Store → VFS → Tonk
broadcast → Remote clients. TipTap JSON document format for structure preservation.

**Tech Stack:** TipTap (React editor), Zustand (state), Tonk VFS (sync), Lucide React (icons),
Tailwind CSS v4 (styling)

---

## Task 1: Install Dependencies

**Files:**

- Modify: `app/package.json`

**Step 1: Install TipTap and Lucide packages**

```bash
cd app
bun add @tiptap/react @tiptap/starter-kit @tiptap/extension-image @tiptap/extension-link @tiptap/extension-text-align @tiptap/extension-underline @tiptap/extension-color @tiptap/extension-highlight @tiptap/extension-task-list @tiptap/extension-task-item lucide-react
```

Expected: Dependencies installed successfully

**Step 2: Verify installation**

```bash
grep "@tiptap/react" package.json
grep "lucide-react" package.json
```

Expected: Both packages appear in dependencies

**Step 3: Commit**

```bash
git add package.json bun.lock
git commit -m "feat(editor): add TipTap and Lucide dependencies"
```

---

## Task 2: Install TipTap Simple Editor Template

**Files:**

- Create: `app/src/features/editor/components/tiptap/` (directory and contents)

**Step 1: Run TipTap CLI to generate simple-editor template**

```bash
cd app
npx @tiptap/cli@latest add simple-editor
```

Expected: Template files created in default location (likely
`src/components/tiptap-templates/simple/`)

**Step 2: Move template to feature directory**

```bash
mkdir -p src/features/editor/components/tiptap
# Move generated files to features/editor/components/tiptap/
# CLI typically creates: simple-editor.tsx, toolbar components, extensions, etc.
```

Note: Exact structure depends on CLI output. Adjust paths as needed.

**Step 3: Verify template structure**

```bash
ls -la src/features/editor/components/tiptap/
```

Expected: See simple-editor.tsx and related component files

**Step 4: Commit**

```bash
git add src/features/editor/components/tiptap/
git commit -m "feat(editor): add TipTap simple-editor template"
```

---

## Task 3: Create Editor Store

**Files:**

- Create: `app/src/features/editor/stores/editorStore.ts`

**Step 1: Create stores directory**

```bash
mkdir -p src/features/editor/stores
```

**Step 2: Write editorStore.ts**

Create `src/features/editor/stores/editorStore.ts`:

```typescript
import { create } from 'zustand';
import { sync } from '../../../lib/middleware';
import type { JSONContent } from '@tiptap/react';

interface EditorState {
  document: JSONContent | null;
  setDocument: (doc: JSONContent) => void;
  clearDocument: () => void;
}

export const useEditorStore = create<EditorState>()(
  sync(
    set => ({
      document: null,

      setDocument: (doc: JSONContent) => {
        set({ document: doc });
      },

      clearDocument: () => {
        set({ document: null });
      },
    }),
    {
      path: '/src/stores/editor.json',
    }
  )
);
```

**Step 3: Verify TypeScript compiles**

```bash
cd app
bun run build
```

Expected: No TypeScript errors related to editorStore

**Step 4: Commit**

```bash
git add src/features/editor/stores/editorStore.ts
git commit -m "feat(editor): create editor store with Tonk sync"
```

---

## Task 4: Replace Icons with Lucide (Part 1 - Core Icons)

**Files:**

- Modify: Icon components in `app/src/features/editor/components/tiptap/`

**Note:** The exact files depend on CLI output. This task provides the mapping pattern.

**Step 1: Identify icon files**

```bash
grep -r "icon" src/features/editor/components/tiptap/ --include="*.tsx"
```

**Step 2: Create icon mapping reference**

Common replacements:

- Template bold icon → `import { Bold } from 'lucide-react'`
- Template italic icon → `import { Italic } from 'lucide-react'`
- Template heading icon → `import { Heading1, Heading2 } from 'lucide-react'`
- Template list icon → `import { List, ListOrdered, ListTodo } from 'lucide-react'`
- Template align icon →
  `import { AlignLeft, AlignCenter, AlignRight, AlignJustify } from 'lucide-react'`

**Step 3: Replace in toolbar button components**

Example pattern for a mark button component:

```typescript
// Before
import { BoldIcon } from './icons/bold-icon'

// After
import { Bold } from 'lucide-react'

// In component
<Bold className="h-4 w-4" />
```

**Step 4: Test build**

```bash
bun run build
```

Expected: No errors, icons render correctly

**Step 5: Commit**

```bash
git add src/features/editor/components/tiptap/
git commit -m "feat(editor): replace template icons with Lucide (core formatting)"
```

---

## Task 5: Replace Icons with Lucide (Part 2 - Advanced Icons)

**Files:**

- Continue modifying: Icon components in `app/src/features/editor/components/tiptap/`

**Step 1: Replace link and media icons**

```typescript
import { Link, Unlink, Image, Quote, Code, Highlighter } from 'lucide-react';
```

**Step 2: Replace history icons**

```typescript
import { Undo, Redo } from 'lucide-react';
```

**Step 3: Replace theme icons**

```typescript
import { Sun, Moon } from 'lucide-react';
```

**Step 4: Test build**

```bash
bun run build
```

Expected: No errors

**Step 5: Commit**

```bash
git add src/features/editor/components/tiptap/
git commit -m "feat(editor): replace remaining icons with Lucide"
```

---

## Task 6: Create Main Editor Component

**Files:**

- Create: `app/src/features/editor/components/editor.tsx`

**Step 1: Write editor.tsx**

Create `src/features/editor/components/editor.tsx`:

```typescript
import { useEditor, EditorContent } from '@tiptap/react';
import { useEffect } from 'react';
import StarterKit from '@tiptap/starter-kit';
import Image from '@tiptap/extension-image';
import Link from '@tiptap/extension-link';
import TextAlign from '@tiptap/extension-text-align';
import Underline from '@tiptap/extension-underline';
import TextStyle from '@tiptap/extension-text-style';
import Color from '@tiptap/extension-color';
import Highlight from '@tiptap/extension-highlight';
import TaskList from '@tiptap/extension-task-list';
import TaskItem from '@tiptap/extension-task-item';
import { useEditorStore } from '../stores/editorStore';
import { SimpleEditor } from './tiptap/simple-editor';

export function Editor() {
  const { document, setDocument } = useEditorStore();

  const editor = useEditor({
    extensions: [
      StarterKit,
      Image,
      Link.configure({
        openOnClick: false,
        HTMLAttributes: {
          class: 'text-blue-600 underline',
        },
      }),
      TextAlign.configure({
        types: ['heading', 'paragraph'],
      }),
      Underline,
      TextStyle,
      Color,
      Highlight,
      TaskList,
      TaskItem.configure({
        nested: true,
      }),
    ],
    content: document || {
      type: 'doc',
      content: [
        {
          type: 'paragraph',
          content: [
            {
              type: 'text',
              text: 'Start typing to see collaborative editing in action...',
            },
          ],
        },
      ],
    },
    onUpdate: ({ editor }) => {
      const json = editor.getJSON();
      setDocument(json);
    },
  });

  // Sync remote updates from store to TipTap
  useEffect(() => {
    if (editor && document) {
      const currentContent = JSON.stringify(editor.getJSON());
      const newContent = JSON.stringify(document);

      // Only update if content actually changed (prevents infinite loops)
      if (currentContent !== newContent) {
        editor.commands.setContent(document);
      }
    }
  }, [document, editor]);

  if (!editor) {
    return <div>Loading editor...</div>;
  }

  return <SimpleEditor editor={editor} />;
}
```

**Step 2: Test build**

```bash
bun run build
```

Expected: No TypeScript errors

**Step 3: Commit**

```bash
git add src/features/editor/components/editor.tsx
git commit -m "feat(editor): create main editor component with sync"
```

---

## Task 7: Update Feature Index

**Files:**

- Modify: `app/src/features/editor/index.tsx`

**Step 1: Update index.tsx to export Editor**

Replace `src/features/editor/index.tsx`:

```typescript
export { Editor } from './components/editor';
```

**Step 2: Verify App.tsx imports work**

Check `src/App.tsx`:

```typescript
import Editor from './features/editor/editor';
```

Should be:

```typescript
import { Editor } from './features/editor';
```

**Step 3: Update App.tsx if needed**

Modify `src/App.tsx`:

```typescript
import './App.css';
import Layout from './components/layout';
import { Editor } from './features/editor';

function App() {
  return (
    <Layout>
      <Editor />
    </Layout>
  );
}

export default App;
```

**Step 4: Test build**

```bash
bun run build
```

Expected: Build succeeds

**Step 5: Commit**

```bash
git add src/features/editor/index.tsx src/App.tsx
git commit -m "feat(editor): export Editor from feature index"
```

---

## Task 8: Fix SimpleEditor Component Integration

**Files:**

- Modify: `app/src/features/editor/components/tiptap/simple-editor.tsx`

**Step 1: Review SimpleEditor props**

Open `src/features/editor/components/tiptap/simple-editor.tsx` and verify it accepts an `editor`
prop of type `Editor | null` from TipTap.

**Step 2: Ensure SimpleEditor uses the provided editor instance**

The component should NOT call `useEditor()` internally. It should receive the editor instance as a
prop.

Expected structure:

```typescript
import type { Editor } from '@tiptap/react';

interface SimpleEditorProps {
  editor: Editor | null;
}

export function SimpleEditor({ editor }: SimpleEditorProps) {
  if (!editor) {
    return null;
  }

  return (
    <div className="editor-container">
      {/* Toolbar using editor instance */}
      <EditorContent editor={editor} />
    </div>
  );
}
```

**Step 3: Fix if needed**

If SimpleEditor tries to create its own editor instance, modify it to accept the editor as a prop
instead.

**Step 4: Test build**

```bash
bun run build
```

Expected: No errors

**Step 5: Commit (if changes made)**

```bash
git add src/features/editor/components/tiptap/simple-editor.tsx
git commit -m "fix(editor): ensure SimpleEditor uses provided editor instance"
```

---

## Task 9: Manual Testing - Single User

**Files:**

- None (testing task)

**Step 1: Start development server**

```bash
cd app
bun run dev
```

Expected: Server starts on http://localhost:4000 (or configured port)

**Step 2: Open browser**

Navigate to: http://localhost:4000

**Step 3: Test basic editing**

1. Type some text in the editor
2. Verify text appears
3. Try bold formatting (Ctrl+B or toolbar button)
4. Verify bold works
5. Try italic formatting
6. Try creating a list
7. Try adding a heading

Expected: All formatting tools work, editor is functional

**Step 4: Check console for errors**

Open browser DevTools → Console

Expected: No errors related to TipTap or editor

**Step 5: Verify VFS sync**

In console, check for:

- "Sync middleware initializing for path: /src/stores/editor.json"
- "Loaded and merged state from /src/stores/editor.json"

Expected: Store is syncing with Tonk VFS

---

## Task 10: Manual Testing - Collaborative Editing

**Files:**

- None (testing task)

**Step 1: Open first browser tab**

Navigate to: http://localhost:4000

Type some text: "Hello from tab 1"

**Step 2: Open second browser tab**

Open a new tab to: http://localhost:4000

Expected: Should see "Hello from tab 1" appear in second tab

**Step 3: Test bidirectional sync**

In tab 2, type: " - and hello from tab 2"

Switch to tab 1

Expected: See the new text from tab 2 appear

**Step 4: Test simultaneous editing**

1. Position cursor at different locations in each tab
2. Type in both tabs at the same time
3. Observe behavior

Expected: Text appears in both tabs (may see last-write-wins behavior)

**Step 5: Test formatting sync**

1. In tab 1, select text and make it bold
2. Switch to tab 2

Expected: Bold formatting appears in tab 2

**Step 6: Document any issues**

If conflicts or sync issues occur, note them for potential future improvements (debouncing, CRDT,
etc.)

---

## Task 11: Final Commit and Cleanup

**Files:**

- All modified files

**Step 1: Run final build**

```bash
cd app
bun run build
```

Expected: Build succeeds with no errors

**Step 2: Check git status**

```bash
git status
```

Expected: All changes committed (or only intentional changes remaining)

**Step 3: Final commit if needed**

If any remaining changes:

```bash
git add .
git commit -m "feat(editor): complete TipTap collaborative editor integration"
```

**Step 4: Test one more time**

Start dev server and verify everything works

**Step 5: Document completion**

Mark the design document as implemented by updating status:

Edit `docs/plans/2025-10-23-tiptap-collaborative-editor-design.md`:

Change:

```markdown
**Status**: Approved
```

To:

```markdown
**Status**: Implemented
```

**Step 6: Commit status update**

```bash
git add docs/plans/2025-10-23-tiptap-collaborative-editor-design.md
git commit -m "docs(editor): mark design as implemented"
```

---

## Success Criteria Verification

After completing all tasks, verify:

- ✅ Editor renders with TipTap simple-editor UI
- ✅ All formatting tools work (bold, italic, lists, headings, etc.)
- ✅ Typing in one browser tab appears in another tab
- ✅ Lucide icons used throughout
- ✅ Follows bulletproof-react structure (`features/editor/`)
- ✅ Uses existing Tonk sync middleware pattern
- ✅ No console errors
- ✅ Store syncs via VFS to `/src/stores/editor.json`

---

## Troubleshooting Common Issues

### Issue: TipTap editor not rendering

**Check:**

1. Browser console for errors
2. Editor instance is not null
3. Extensions are imported correctly
4. SimpleEditor component receives editor prop

### Issue: Sync not working between tabs

**Check:**

1. Tonk relay is running
2. VFS middleware initialized (check console logs)
3. Store path is correct: `/src/stores/editor.json`
4. Both tabs connected to same relay

### Issue: Infinite update loop

**Check:**

1. JSON comparison in useEffect is working
2. onUpdate only fires on user edits, not programmatic updates
3. Consider adding transaction origin checking

### Issue: Icons not displaying

**Check:**

1. lucide-react is installed
2. Icon imports are correct
3. Tailwind classes are applied (h-4 w-4 or similar)
4. Icon components are used correctly

---

## Related Documentation

- Design: `docs/plans/2025-10-23-tiptap-collaborative-editor-design.md`
- Reference implementation: `src/stores/pixelStore.ts`
- Middleware: `src/lib/middleware.ts`
- TipTap docs: https://tiptap.dev/docs/editor/getting-started/install/react
- TipTap simple editor: https://tiptap.dev/docs/ui-components/templates/simple-editor
