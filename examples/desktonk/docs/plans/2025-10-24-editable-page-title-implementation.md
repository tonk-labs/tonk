# Editable Page Title Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan
> task-by-task.

**Goal:** Add click-to-edit page title centered in header, synced to VFS with document metadata.

**Architecture:** Extend editorStore with metadata field, create EditableTitle component with
click-to-edit interaction, update header to three-column flex layout for perfect centering.

**Tech Stack:** React, TypeScript, Zustand, CSS modules, VFS sync via existing middleware

---

## Task 1: Extend Editor Store with Metadata

**Files:**

- Modify: `app/src/features/editor/stores/editorStore.ts:1-25`

**Step 1: Update the EditorState interface**

Add metadata field to the store interface:

```typescript
interface EditorState {
  document: JSONContent | null;
  metadata: {
    title: string;
  };
  setDocument: (doc: JSONContent) => void;
  setTitle: (title: string) => void;
  setMetadata: (metadata: { title: string }) => void;
  clearDocument: () => void;
}
```

**Step 2: Add metadata to initial state**

Update the store implementation:

```typescript
export const useEditorStore = create<EditorState>()(set => ({
  document: null,
  metadata: {
    title: 'Untitled',
  },

  setDocument: (doc: JSONContent) => {
    set({ document: doc });
  },

  setTitle: (title: string) => {
    set(state => ({
      metadata: { ...state.metadata, title },
    }));
  },

  setMetadata: (metadata: { title: string }) => {
    set({ metadata });
  },

  clearDocument: () => {
    set({ document: null, metadata: { title: 'Untitled' } });
  },
}));
```

**Step 3: Verify store compiles**

Run: `bun run build` Expected: Build succeeds with no TypeScript errors

**Step 4: Commit**

```bash
git add app/src/features/editor/stores/editorStore.ts
git commit -m "feat(store): add metadata field with title to editorStore"
```

---

## Task 2: Create EditableTitle Component

**Files:**

- Create: `app/src/components/header/editable-title.tsx`
- Create: `app/src/components/header/editable-title.css`

**Step 1: Create the TypeScript component file**

Create `app/src/components/header/editable-title.tsx`:

```typescript
import { useState, useRef, useEffect, KeyboardEvent } from 'react';
import { useEditorStore } from '@/features/editor/stores/editorStore';
import './editable-title.css';

const MAX_TITLE_LENGTH = 100;

export function EditableTitle() {
  const title = useEditorStore(state => state.metadata.title);
  const setTitle = useEditorStore(state => state.setTitle);
  const [isEditing, setIsEditing] = useState(false);
  const [localTitle, setLocalTitle] = useState(title);
  const inputRef = useRef<HTMLInputElement>(null);

  // Sync local state when title changes externally
  useEffect(() => {
    if (!isEditing) {
      setLocalTitle(title);
    }
  }, [title, isEditing]);

  // Auto-focus and select text when entering edit mode
  useEffect(() => {
    if (isEditing && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [isEditing]);

  const validateAndSave = () => {
    let validated = localTitle.trim();

    // Enforce max length
    if (validated.length > MAX_TITLE_LENGTH) {
      validated = validated.substring(0, MAX_TITLE_LENGTH);
    }

    // Revert to "Untitled" if empty
    if (validated === '') {
      validated = 'Untitled';
    }

    setTitle(validated);
    setLocalTitle(validated);
    setIsEditing(false);
  };

  const handleKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      validateAndSave();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      setLocalTitle(title);
      setIsEditing(false);
    }
  };

  const handleClick = () => {
    if (!isEditing) {
      setIsEditing(true);
    }
  };

  const handleBlur = () => {
    validateAndSave();
  };

  if (isEditing) {
    return (
      <input
        ref={inputRef}
        type="text"
        value={localTitle}
        onChange={(e) => setLocalTitle(e.target.value)}
        onKeyDown={handleKeyDown}
        onBlur={handleBlur}
        className="editable-title-input"
        aria-label="Document title"
        maxLength={MAX_TITLE_LENGTH + 10} // Allow some buffer for user experience
      />
    );
  }

  return (
    <h1
      onClick={handleClick}
      className="editable-title-display"
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          handleClick();
        }
      }}
    >
      {title}
    </h1>
  );
}
```

**Step 2: Create the CSS file**

Create `app/src/components/header/editable-title.css`:

```css
.editable-title-display {
  font-size: 1rem;
  font-weight: 500;
  color: var(--tt-gray-light-900, #1d1e20);
  margin: 0;
  padding: 0.375rem 0.75rem;
  cursor: pointer;
  border-radius: 0.25rem;
  transition: background-color 0.15s ease;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 400px;
  text-align: center;
}

.editable-title-display:hover {
  background-color: var(--tt-gray-light-a-100, rgba(15, 22, 36, 0.05));
}

.editable-title-display:focus-visible {
  outline: 2px solid var(--tt-gray-light-400, rgba(166, 167, 171, 1));
  outline-offset: 2px;
}

.editable-title-input {
  font-size: 1rem;
  font-weight: 500;
  color: var(--tt-gray-light-900, #1d1e20);
  margin: 0;
  padding: 0.375rem 0.75rem;
  border: 2px solid var(--tt-gray-light-400, rgba(166, 167, 171, 1));
  border-radius: 0.25rem;
  background-color: white;
  text-align: center;
  max-width: 400px;
  min-width: 200px;
}

.editable-title-input:focus {
  outline: none;
  border-color: var(--tt-gray-light-600, rgba(36, 39, 46, 0.78));
}
```

**Step 3: Verify component compiles**

Run: `bun run build` Expected: Build succeeds with no TypeScript errors

**Step 4: Commit**

```bash
git add app/src/components/header/editable-title.tsx app/src/components/header/editable-title.css
git commit -m "feat(header): create EditableTitle component with click-to-edit"
```

---

## Task 3: Update Header Layout

**Files:**

- Modify: `app/src/components/header/header.tsx:1-24`

**Step 1: Import EditableTitle component**

Update imports at the top of `header.tsx`:

```typescript
import { DownloadIcon, Share2Icon } from 'lucide-react';
import { Button } from '../ui/button/button';
import { PresenceIndicators } from '@/features/presence';
import { EditableTitle } from './editable-title';
```

**Step 2: Update header structure to three-column layout**

Replace the entire component:

```typescript
export default function Header() {
  return (
    <nav className="flex items-center justify-between w-full">
      {/* Left spacer - ensures center alignment */}
      <div className="flex-1" />

      {/* Center - Editable title */}
      <div className="flex-1 flex justify-center">
        <EditableTitle />
      </div>

      {/* Right - buttons and presence */}
      <div className="flex-1 flex justify-end items-center gap-2">
        <PresenceIndicators maxVisible={5} />
        <Button>
          <DownloadIcon />
        </Button>
        <Button>
          Share
          <Share2Icon />
        </Button>
      </div>
    </nav>
  );
}
```

**Step 3: Verify header renders correctly**

Run: `bun run dev` Expected: Dev server starts, navigate to http://localhost:4000, header shows
centered "Untitled" title

**Step 4: Test click-to-edit interaction**

Manual test:

1. Click on "Untitled" → Should become input field with text selected
2. Type "Test Document" → Input should update
3. Press Enter → Should save and exit edit mode
4. Click title again → Should re-enter edit mode
5. Press Escape → Should cancel and keep previous value

**Step 5: Commit**

```bash
git add app/src/components/header/header.tsx
git commit -m "feat(header): integrate EditableTitle with three-column layout"
```

---

## Task 4: Add Title Validation Tests

**Files:**

- Create: `app/src/components/header/__tests__/editable-title.test.tsx`

**Note:** This project doesn't currently have a test setup. This task documents what tests SHOULD be
written when a testing framework (like Vitest) is added.

**Future Test Cases to Implement:**

```typescript
import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { EditableTitle } from '../editable-title';

describe('EditableTitle', () => {
  it('displays current title from store', () => {
    render(<EditableTitle />);
    expect(screen.getByText('Untitled')).toBeInTheDocument();
  });

  it('enters edit mode on click', () => {
    render(<EditableTitle />);
    fireEvent.click(screen.getByText('Untitled'));
    expect(screen.getByRole('textbox')).toBeInTheDocument();
  });

  it('saves on Enter key', () => {
    render(<EditableTitle />);
    fireEvent.click(screen.getByText('Untitled'));
    const input = screen.getByRole('textbox');
    fireEvent.change(input, { target: { value: 'New Title' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(screen.getByText('New Title')).toBeInTheDocument();
  });

  it('reverts to "Untitled" when empty', () => {
    render(<EditableTitle />);
    fireEvent.click(screen.getByText('Untitled'));
    const input = screen.getByRole('textbox');
    fireEvent.change(input, { target: { value: '   ' } });
    fireEvent.blur(input);
    expect(screen.getByText('Untitled')).toBeInTheDocument();
  });

  it('truncates titles over 100 characters', () => {
    const longTitle = 'a'.repeat(120);
    render(<EditableTitle />);
    fireEvent.click(screen.getByText('Untitled'));
    const input = screen.getByRole('textbox');
    fireEvent.change(input, { target: { value: longTitle } });
    fireEvent.blur(input);
    expect(screen.getByText('a'.repeat(100))).toBeInTheDocument();
  });

  it('trims whitespace on save', () => {
    render(<EditableTitle />);
    fireEvent.click(screen.getByText('Untitled'));
    const input = screen.getByRole('textbox');
    fireEvent.change(input, { target: { value: '  My Title  ' } });
    fireEvent.blur(input);
    expect(screen.getByText('My Title')).toBeInTheDocument();
  });
});
```

**Step 1: Document test requirements**

Create placeholder file showing what should be tested:

````bash
mkdir -p app/src/components/header/__tests__
cat > app/src/components/header/__tests__/README.md << 'EOF'
# EditableTitle Tests

## Required Test Coverage

- Display current title from store
- Enter edit mode on click
- Save on Enter key
- Cancel on Escape key
- Revert to "Untitled" when empty after trim
- Truncate titles over 100 characters
- Trim whitespace on save
- Auto-focus and select text in edit mode
- Keyboard navigation support

## Setup Required

Install Vitest and React Testing Library:
```bash
bun add -d vitest @testing-library/react @testing-library/user-event jsdom
````

Add test script to package.json:

```json
"test": "vitest"
```

EOF

````

**Step 2: Commit test documentation**

```bash
git add app/src/components/header/__tests__/README.md
git commit -m "docs(tests): document EditableTitle test requirements"
````

---

## Task 5: Enable VFS Sync for Metadata

**Files:**

- Modify: `app/src/features/editor/stores/editorStore.ts:1-25`
- Modify: `app/src/lib/middleware.ts:1-219`

**Step 1: Re-enable sync middleware in editorStore**

Update `editorStore.ts` to use sync middleware:

```typescript
import { create } from 'zustand';
import { sync } from '../../../lib/middleware';
import type { JSONContent } from '@tiptap/react';

interface EditorState {
  document: JSONContent | null;
  metadata: {
    title: string;
  };
  setDocument: (doc: JSONContent) => void;
  setTitle: (title: string) => void;
  setMetadata: (metadata: { title: string }) => void;
  clearDocument: () => void;
}

export const useEditorStore = create<EditorState>()(
  sync(
    set => ({
      document: null,
      metadata: {
        title: 'Untitled',
      },

      setDocument: (doc: JSONContent) => {
        set({ document: doc });
      },

      setTitle: (title: string) => {
        set(state => ({
          metadata: { ...state.metadata, title },
        }));
      },

      setMetadata: (metadata: { title: string }) => {
        set({ metadata });
      },

      clearDocument: () => {
        set({ document: null, metadata: { title: 'Untitled' } });
      },
    }),
    { path: '/documents/current.json' }
  )
);
```

**Step 2: Verify middleware serializes metadata correctly**

The existing middleware (middleware.ts:41-44) already serializes the entire state, which will
include our metadata field.

Check that the serialization includes metadata:

```typescript
// In middleware.ts, serializeState already handles this:
const serializeState = (state: T): any => {
  const serializable = JSON.parse(JSON.stringify(state));
  return serializable;
};
```

**Step 3: Test VFS sync manually**

Run: `bun run dev`

Manual test:

1. Wait for VFS to connect (check console for "VFS initialization completed!")
2. Change document title to "Test Document"
3. Open browser DevTools → Application → IndexedDB
4. Verify `/documents/current.json` contains:

```json
{
  "document": null,
  "metadata": {
    "title": "Test Document"
  }
}
```

**Step 4: Commit VFS integration**

```bash
git add app/src/features/editor/stores/editorStore.ts
git commit -m "feat(sync): enable VFS sync for editor metadata"
```

---

## Task 6: Test Real-Time Sync Across Tabs

**Files:**

- None (manual testing)

**Step 1: Open two browser tabs**

1. Run: `bun run dev`
2. Open http://localhost:4000 in first tab
3. Open http://localhost:4000 in second tab

**Step 2: Verify title syncs between tabs**

Manual test:

1. In Tab 1: Click title, change to "Shared Document"
2. In Tab 2: Verify title updates to "Shared Document" within 1-2 seconds
3. In Tab 2: Click title, change to "Updated Title"
4. In Tab 1: Verify title updates to "Updated Title" within 1-2 seconds

Expected: Title changes sync in real-time across both tabs via VFS watchers

**Step 3: Test concurrent edits**

Manual test:

1. In Tab 1: Start editing title (click, don't save yet)
2. In Tab 2: Change title to "Final Title"
3. In Tab 1: Finish editing with different value
4. Expected: Last save wins (Tab 1's value persists)

**Step 4: Document sync behavior**

If sync works correctly, proceed. If issues found, investigate middleware.ts:100-113 where file
watchers update state.

---

## Task 7: Final Verification and Documentation

**Files:**

- Modify: `README.md` (add feature documentation)

**Step 1: Verify all requirements met**

Checklist:

- ✓ Title centered in header
- ✓ Click to edit interaction
- ✓ Auto-focus and select text
- ✓ Enter saves, Escape cancels
- ✓ Max 100 characters enforced
- ✓ Empty titles revert to "Untitled"
- ✓ Whitespace trimmed
- ✓ VFS sync enabled
- ✓ Real-time updates across tabs

**Step 2: Update README with feature description**

Add to README.md:

```markdown
## Features

### Editable Page Title

- Click the centered page title in the header to edit
- Press Enter to save or Escape to cancel
- Titles are automatically trimmed and limited to 100 characters
- Empty titles default to "Untitled"
- Title changes sync in real-time across all connected clients via VFS
- Title is stored as metadata alongside document content

#### Storage Format

Document metadata is stored in the VFS as:

\`\`\`json { "content": { /_ TipTap document JSON _/ }, "metadata": { "title": "Document Title" } }
\`\`\`

This structure supports future metadata extensions (tags, description, author, etc.).
```

**Step 3: Final build verification**

Run: `bun run build` Expected: Clean build with no errors or warnings

**Step 4: Final commit**

```bash
git add README.md
git commit -m "docs: document editable page title feature"
```

**Step 5: Create pull request**

```bash
git push -u origin ramram/feat/berlin
gh pr create --title "feat: Add editable page title with VFS sync" --body "Implements centered, editable page title in header with real-time VFS synchronization.

## Changes
- Extended editorStore with metadata field
- Created EditableTitle component with click-to-edit interaction
- Updated header to three-column layout for centering
- Enabled VFS sync for title metadata
- Added validation (max 100 chars, trim whitespace, revert empty to 'Untitled')

## Testing
- Manual testing of edit interactions
- Manual testing of VFS sync across tabs
- Test documentation added for future test suite

See docs/plans/2025-10-24-editable-page-title-design.md for full design."
```

---

## Completion Checklist

- [ ] Task 1: Editor store extended with metadata
- [ ] Task 2: EditableTitle component created
- [ ] Task 3: Header layout updated
- [ ] Task 4: Test requirements documented
- [ ] Task 5: VFS sync enabled
- [ ] Task 6: Real-time sync verified
- [ ] Task 7: Documentation updated and PR created

## Notes

- Project currently lacks test setup; Task 4 documents required tests for future implementation
- VFS sync commented out in current code; Task 5 re-enables it
- Middleware already handles serialization correctly; no changes needed
- Consider adding Vitest for proper test coverage in future
