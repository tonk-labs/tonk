# Unified Editor Refactoring Plan

## Overview

Consolidate the current multiple editor implementations (ComponentEditor, StoreEditor, PageEditor)
into a single unified editor with Preview/Editor tabs and Monaco editor integration.

## Current State Analysis

### Existing Editor Implementations

1. **ComponentEditor** (`src/components/ComponentEditor.tsx`)
   - Basic textarea-based editor
   - Auto-save functionality
   - Inline error display
   - File path: `/src/components/`

2. **StoreEditor** (`src/components/StoreEditor.tsx`)
   - Similar to ComponentEditor
   - TypeScript compilation status indicator
   - Store-specific template hints
   - File path: `/src/stores/`

3. **PageEditor** (`src/components/PageEditor.tsx`)
   - Has Preview/Edit tabs already
   - Basic textarea editor
   - ViewRenderer for preview
   - File path: `/src/views/`

4. **ComponentManager** (`src/components/ComponentManager.tsx`)
   - Uses ComponentEditor + ComponentPreview
   - Has Preview/Edit tabs
   - Template system for new components

### Common Features Across Editors

- Auto-save with debouncing
- Error display (compilation/VFS errors)
- File browser sidebar
- Status indicators
- Basic textarea for code editing

### Pain Points

- Code duplication across editors
- No proper syntax highlighting (using textarea)
- Different implementations for similar functionality
- No Monaco editor integration

## Proposed Solution

### 1. Unified Editor Component Structure

```
src/components/unified-editor/
├── UnifiedEditor.tsx           # Main component with tabs
├── MonacoCodeEditor.tsx       # Monaco editor wrapper
├── PreviewPane.tsx            # Unified preview component
├── FileBrowser.tsx            # Generic file browser
├── EditorHeader.tsx           # Common header with status
└── hooks/
    ├── useMonacoEditor.ts     # Monaco initialization
    ├── useFileOperations.ts   # VFS operations
    └── useCompilation.ts      # TypeScript compilation
```

### 2. Component Architecture

#### UnifiedEditor.tsx

Main container component with:

- Tab switching (Preview/Editor)
- Props for `editorOnly` mode
- File type context (component/store/page/generic)
- Layout management

#### MonacoCodeEditor.tsx

Monaco editor integration:

- Syntax highlighting for TSX/TS
- IntelliSense support
- Theme configuration
- Error markers from TypeScript validation
- Auto-save integration

#### PreviewPane.tsx

Unified preview using existing logic:

- ComponentPreview for components
- ViewRenderer for pages
- Store state display for stores
- Error boundary for safe rendering

#### FileBrowser.tsx

Generic file browser with:

- Directory filtering based on editor type
- Search functionality
- Create/Delete operations
- File selection callbacks

### 3. Implementation Steps

#### Phase 1: Setup Monaco Editor

1. Install @monaco-editor/react package
2. Create MonacoCodeEditor wrapper component
3. Configure TypeScript language service
4. Setup themes and editor options

#### Phase 2: Create Unified Components

1. Build UnifiedEditor container
2. Implement tab switching logic
3. Create EditorHeader with status indicators
4. Port auto-save functionality

#### Phase 3: Consolidate File Browser

1. Extract common file browser logic
2. Add filtering by file type
3. Implement search and CRUD operations
4. Handle file selection

#### Phase 4: Unify Preview Logic

1. Create PreviewPane component
2. Integrate ComponentPreview
3. Add ViewRenderer support
4. Handle store previews

#### Phase 5: Update Routes

1. Modify Editor.tsx to use UnifiedEditor
2. Pass appropriate props for each route:
   - `/editor/components` → fileFilter: `/src/components`
   - `/editor/stores` → fileFilter: `/src/stores`
   - `/editor/pages` → fileFilter: `/src/views`
   - `/editor/files` → fileFilter: null (all files)

#### Phase 6: Cleanup

1. Remove old editor components
2. Update imports
3. Test all editor modes
4. Ensure backward compatibility

### 4. Configuration Options

```typescript
interface UnifiedEditorProps {
  fileFilter?: string; // Directory to filter files
  editorOnly?: boolean; // Hide preview tab
  defaultTab?: 'preview' | 'editor';
  onFileChange?: (path: string) => void;
  height?: string;
  debounceDelay?: number;
}
```

### 5. Monaco Editor Configuration

```typescript
const monacoOptions = {
  theme: 'vs-dark',
  language: 'typescript',
  fontSize: 14,
  minimap: { enabled: false },
  automaticLayout: true,
  formatOnPaste: true,
  formatOnType: true,
  suggestOnTriggerCharacters: true,
  quickSuggestions: {
    other: true,
    comments: false,
    strings: true,
  },
};
```

### 6. File Type Detection

```typescript
const getEditorType = (filePath: string) => {
  if (filePath.startsWith('/src/components/')) return 'component';
  if (filePath.startsWith('/src/stores/')) return 'store';
  if (filePath.startsWith('/src/views/')) return 'page';
  return 'generic';
};
```

### 7. Benefits of This Approach

- Single source of truth for editor logic
- Consistent UX across all editor types
- Better code editing experience with Monaco
- Reduced code duplication
- Easier to maintain and extend
- Proper TypeScript/TSX support

### 8. Migration Strategy

1. Build new components alongside existing ones
2. Test thoroughly with existing files
3. Gradually replace old editors
4. Keep old components temporarily for rollback
5. Remove old components after validation

### 9. Testing Plan

- Test component creation and editing
- Verify store compilation and updates
- Check page preview and routing
- Test file browser operations
- Validate auto-save functionality
- Ensure error handling works
- Test Monaco features (IntelliSense, formatting)

### 10. Dependencies

- @monaco-editor/react (need to install)
- Existing: React, TypeScript compiler, VFS service
- No breaking changes to existing APIs

## Timeline Estimate

- Phase 1-2: 2 hours (Monaco setup + basic structure)
- Phase 3-4: 2 hours (File browser + preview)
- Phase 5-6: 1 hour (Integration + cleanup)
- Testing: 1 hour

Total: ~6 hours of implementation
