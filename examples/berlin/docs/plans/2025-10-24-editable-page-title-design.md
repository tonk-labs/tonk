# Editable Page Title Design

**Date:** 2025-10-24 **Status:** Approved

## Overview

Users need an editable page title centered in the header. The title syncs to the VFS and appears in
the document's frontmatter for future metadata features.

## Requirements

### User Interaction

- Click title to edit (single click, no double-click required)
- Title auto-focuses and selects all text on edit
- Save on blur or Enter key
- Escape cancels and reverts to original value
- Default title: "Untitled"

### Validation

- Max 100 characters, truncate on save
- Trim leading/trailing whitespace
- Revert to "Untitled" if empty after trim

### Real-time Sync

- Changes sync immediately to VFS (when VFS enabled)
- All users viewing the document see title updates in real-time
- Middleware handles sync automatically

## Architecture

### Data Structure

Extend `editorStore` to include metadata:

```typescript
interface EditorState {
  document: JSONContent | null;
  metadata: {
    title: string;
  };
  setDocument: (doc: JSONContent) => void;
  setTitle: (title: string) => void;
  clearDocument: () => void;
}
```

VFS storage format:

```json
{
  "content": {
    /* TipTap JSON */
  },
  "metadata": {
    "title": "Document Title"
  }
}
```

### Component Structure

**Header Layout (header.tsx:6-22)**

Three-column flex layout centers the title:

```tsx
<nav className="flex items-center justify-between">
  <div className="flex-1" />
  <div className="flex-1 flex justify-center">
    <EditableTitle />
  </div>
  <div className="flex-1 flex justify-end gap-2">
    <PresenceIndicators />
    <Button>
      <DownloadIcon />
    </Button>
    <Button>
      Share <Share2Icon />
    </Button>
  </div>
</nav>
```

**EditableTitle Component (new file)**

The component manages edit mode and validation:

```tsx
const [isEditing, setIsEditing] = useState(false);
const [localTitle, setLocalTitle] = useState('');
const title = useEditorStore(state => state.metadata.title);
const setTitle = useEditorStore(state => state.setTitle);
```

Display mode renders as styled text. Edit mode renders as input field. Local state prevents VFS sync
on every keystroke. Store updates only on blur or Enter.

### State Flow

1. User clicks title → Component enters edit mode
2. User types → Local state updates, no store changes
3. User presses Enter or clicks away → Validate and update store
4. Store update → Middleware syncs to VFS
5. VFS change → Other users see updated title via watcher

### Error Handling

**VFS Connection States**

- Component works offline with local state only
- Middleware handles sync when VFS connects/reconnects
- Connection status visible via existing PresenceIndicators

**Edge Cases**

- Concurrent edits: Last write wins (VFS behavior)
- Long titles: CSS truncates with ellipsis in display mode
- Rapid clicks: `isEditing` check prevents re-entry
- Initial load: Show "Untitled" immediately, update from VFS when loaded

**Accessibility**

- Input includes `aria-label="Document title"`
- Enter saves, Escape cancels (standard behavior)
- Focus management ensures keyboard navigation works

## Implementation Plan

1. Update `editorStore` to include metadata field
2. Create `EditableTitle` component
3. Update header layout for three-column structure
4. Add title validation logic
5. Test VFS sync and real-time updates
6. Verify accessibility with keyboard navigation

## Future Extensions

This design supports future frontmatter features:

- Tags, description, author
- Creation/modification dates
- Custom metadata fields

All metadata lives in the same structure, making expansion simple.
