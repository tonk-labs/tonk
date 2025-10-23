# TipTap Collaborative Editor Design

**Date**: 2025-10-23 **Author**: Claude Code **Status**: Approved

## Overview

Design for integrating TipTap's simple-editor template into the Berlin example app as a
collaborative editing demo. The editor will use Tonk's networking capabilities via the existing
Zustand sync middleware pattern to broadcast document changes in real-time across connected clients.

## Requirements

### Purpose

Demonstrate collaborative editing capabilities using Tonk's VFS synchronization with a full-featured
rich text editor.

### Key Features

- Full TipTap simple-editor template functionality (formatting, lists, images, links, alignment,
  etc.)
- Real-time collaborative editing via Tonk sync middleware
- Bulletproof-react feature structure
- Lucide icons instead of template default icons
- Tailwind CSS v4 styling (already configured)

### Constraints

- Must follow existing `pixelStore.ts` + sync middleware pattern
- Last-write-wins conflict resolution (simple approach for demo)
- Feature-based organization under `features/editor/`

## Architecture

### High-Level Flow

```
User types → TipTap onUpdate → Store action → Zustand state update
  → sync middleware → VFS write → Tonk broadcast
  → Other clients receive → VFS update → Store update
  → TipTap re-render
```

### Component Structure

Following bulletproof-react pattern:

```
features/editor/
├── index.tsx                           # Main entrypoint (exports Editor)
├── components/
│   ├── editor.tsx                     # Main editor component
│   └── tiptap/                        # TipTap template components
│       ├── simple-editor.tsx          # Core editor UI
│       ├── toolbar.tsx                # Toolbar with formatting controls
│       ├── hooks/                     # TipTap hooks
│       ├── extensions/                # Custom extensions
│       ├── nodes/                     # Node components
│       └── primitives/                # UI primitives (buttons, etc.)
├── stores/
│   └── editorStore.ts                 # Zustand + Tonk sync
└── hooks/                              # (optional)
    └── useTiptapSync.ts               # Sync logic if needed
```

### Data Model

**editorStore.ts**:

```typescript
interface EditorState {
  document: JSONContent | null; // TipTap's JSON document format
  setDocument: (doc: JSONContent) => void;
  clearDocument: () => void;
}
```

**Store implementation**:

- Uses `sync()` middleware from `lib/middleware.ts`
- VFS path: `/src/stores/editor.json`
- Follows exact same pattern as `pixelStore.ts`

### Synchronization Strategy

**TipTap → Store (Local Edits)**:

```typescript
const editor = useEditor({
  extensions: [...],
  content: store.document,
  onUpdate: ({ editor }) => {
    const json = editor.getJSON();
    store.setDocument(json);
  }
})
```

**Store → TipTap (Remote Updates)**:

```typescript
useEffect(() => {
  if (editor && store.document) {
    // Prevent infinite loops - only update if content changed
    if (JSON.stringify(editor.getJSON()) !== JSON.stringify(store.document)) {
      editor.commands.setContent(store.document);
    }
  }
}, [store.document, editor]);
```

**Loop Prevention**:

- Compare JSON stringified content before updating
- Optionally inspect `transaction.origin` to detect remote vs local updates
- Only call `store.setDocument()` on user-initiated changes

## Dependencies

### Required Packages

```json
{
  "@tiptap/react": "latest",
  "@tiptap/starter-kit": "latest",
  "@tiptap/extension-image": "latest",
  "@tiptap/extension-link": "latest",
  "@tiptap/extension-text-align": "latest",
  "@tiptap/extension-underline": "latest",
  "@tiptap/extension-color": "latest",
  "@tiptap/extension-highlight": "latest",
  "@tiptap/extension-task-list": "latest",
  "@tiptap/extension-task-item": "latest",
  "lucide-react": "latest"
}
```

### Installation Approach

**Option 1 - TipTap CLI (Recommended)**:

```bash
npx @tiptap/cli@latest add simple-editor
```

Then move generated components from default location to `features/editor/components/tiptap/`

**Option 2 - Manual**: Install packages individually and manually create component structure based
on simple-editor template.

## Implementation Details

### Store Implementation

Create `features/editor/stores/editorStore.ts`:

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

### Icon Replacement

Replace all TipTap template icons with Lucide React equivalents:

| Template Icon | Lucide Icon                  | Component         |
| ------------- | ---------------------------- | ----------------- |
| Bold          | `Bold`                       | Mark button       |
| Italic        | `Italic`                     | Mark button       |
| Underline     | `Underline`                  | Mark button       |
| Strike        | `Strikethrough`              | Mark button       |
| Heading       | `Heading1`, `Heading2`, etc. | Heading dropdown  |
| List          | `List`                       | List button       |
| Ordered List  | `ListOrdered`                | List button       |
| Task List     | `ListTodo`                   | List button       |
| Align Left    | `AlignLeft`                  | Alignment button  |
| Align Center  | `AlignCenter`                | Alignment button  |
| Align Right   | `AlignRight`                 | Alignment button  |
| Justify       | `AlignJustify`               | Alignment button  |
| Link          | `Link`                       | Link popover      |
| Unlink        | `Unlink`                     | Link popover      |
| Image         | `Image`                      | Image upload      |
| Blockquote    | `Quote`                      | Blockquote button |
| Code          | `Code`                       | Code block        |
| Undo          | `Undo`                       | Undo button       |
| Redo          | `Redo`                       | Redo button       |
| Theme         | `Sun`, `Moon`                | Theme toggle      |
| Highlight     | `Highlighter`                | Color highlight   |

Create icon mapping or directly replace in template components.

### Styling Integration

- TipTap template uses Tailwind CSS (matches existing setup)
- Components are pre-styled and responsive
- Dark/light mode support included
- No additional CSS files required
- Toolbar renders at editor top, content area is scrollable

### Editor Integration

`features/editor/components/editor.tsx`:

```typescript
import { useEditor, EditorContent } from '@tiptap/react';
import { useEditorStore } from '../stores/editorStore';
import { SimpleEditor } from './tiptap/simple-editor';
import { useEffect } from 'react';

export function Editor() {
  const { document, setDocument } = useEditorStore();

  const editor = useEditor({
    extensions: [...], // All simple-editor extensions
    content: document,
    onUpdate: ({ editor }) => {
      const json = editor.getJSON();
      setDocument(json);
    },
  });

  // Sync remote updates
  useEffect(() => {
    if (editor && document) {
      const currentContent = JSON.stringify(editor.getJSON());
      const newContent = JSON.stringify(document);

      if (currentContent !== newContent) {
        editor.commands.setContent(document);
      }
    }
  }, [document, editor]);

  return <SimpleEditor editor={editor} />;
}
```

`features/editor/index.tsx`:

```typescript
export { Editor } from './components/editor';
```

## Error Handling & Edge Cases

### Sync Conflicts

- **Strategy**: Last-write-wins (simple approach)
- **Behavior**: User may see their text briefly, then remote update overwrites
- **Acceptable**: This is a demo showing collaboration - conflicts demonstrate it working
- **Future improvement**: Could add CRDT-based sync (Y.js) for better conflict resolution

### Disconnection Handling

- VFS connection state can be monitored via `vfs.isInitialized()`
- Optional: Add disconnected indicator in toolbar
- Local edits continue to work
- Sync resumes automatically on reconnection

### Empty Document

- Store initializes with `document: null`
- First user gets empty editor
- Subsequent users load existing document from VFS
- TipTap handles null/empty content gracefully

### Large Documents

- TipTap JSON format is reasonably compact
- Sync middleware auto-saves on every keystroke
- If performance issues arise, add debouncing (300ms delay)
- Consider batching updates for very large documents

### Cursor Position

- TipTap preserves cursor position during remote updates by default
- User can continue typing even as remote updates arrive
- Content updates but cursor stays in place

## Testing Strategy

### Manual Testing

1. Open two browser tabs to `http://localhost:4000`
2. Type in tab 1, verify text appears in tab 2
3. Type in tab 2, verify text appears in tab 1
4. Test simultaneous typing (observe conflict behavior)
5. Test all formatting buttons
6. Test disconnect/reconnect (close relay, reopen)

### Test Scenarios

- Single user editing
- Two users editing different parts
- Two users editing same location (conflict)
- Network disconnection and reconnection
- Large document (multiple pages of text)
- Image upload and sync
- Link creation and sync

## Success Criteria

- [ ] Editor renders with full TipTap simple-editor UI
- [ ] All formatting tools work (bold, italic, lists, etc.)
- [ ] Typing in one browser tab appears in another tab
- [ ] Lucide icons used throughout
- [ ] Follows bulletproof-react structure
- [ ] Uses existing Tonk sync middleware pattern
- [ ] No console errors
- [ ] Responsive on mobile
- [ ] Dark/light mode support

## Future Enhancements

### Short-term

- Add debouncing for better performance
- Add connection status indicator
- Add user presence (show who else is editing)
- Add user cursors (show where others are typing)

### Long-term

- Integrate Y.js for CRDT-based conflict resolution
- Add commenting/annotations
- Add version history
- Add document permissions
- Add export functionality (PDF, Markdown, HTML)

## References

- [TipTap Simple Editor Template](https://tiptap.dev/docs/ui-components/templates/simple-editor)
- [TipTap React Documentation](https://tiptap.dev/docs/editor/getting-started/install/react)
- [Bulletproof React](https://github.com/alan2207/bulletproof-react)
- Existing implementation: `src/stores/pixelStore.ts`
- Existing middleware: `src/lib/middleware.ts`
