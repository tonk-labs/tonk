# VFS FileTree Component with react-complex-tree

## Overview

Replace the current `VFSFileBrowser` component with a production-ready file tree using
`react-complex-tree` library.

## Why react-complex-tree?

- **W3C Accessibility Compliant** - Full screen reader support and keyboard navigation
- **Built-in State Management** - Handles expand/collapse state automatically
- **Multi-selection & Drag-and-Drop** - Production features out of the box
- **No peer dependencies** - Clean installation
- **Active maintenance** - Regular updates and good documentation

## ⚠️ CRITICAL ISSUES FOUND IN INITIAL IMPLEMENTATION

### 1. Nested Interactive Elements (Buttons inside Buttons)

**Problem**: React-complex-tree renders tree items as buttons. Adding action buttons (delete,
create) inside causes invalid HTML nesting and React warnings. **Solution**: Use `renderItemArrow`
and `renderItemTitle` separately, or use click handlers on non-button elements with proper event
stopping.

### 2. Missing CSS Classes in Twind

**Problem**: Twind runtime CSS doesn't recognize react-complex-tree classes (`rct-tree-root`,
`rct-tree-item`, etc.) or Lucide icon classes. **Solution**: Either:

- Import react-complex-tree CSS directly in index.html
- Add the classes to Twind's style configuration
- Use inline styles instead of CSS classes

### 3. Folder Expansion Not Working

**Problem**: Clicking folders doesn't expand them - tree state isn't updating properly.
**Solution**: Ensure VFSDataProvider correctly implements `getTreeItem` with proper `children` and
`isFolder` properties.

### 4. Duplicate "users" Folders

**Problem**: VFS is returning duplicate entries for some directories. **Solution**: Debug VFS
service to ensure unique directory listings.

## REVISED Implementation Plan (Fixing Critical Issues)

### Phase 0: Fix Critical Issues First

#### 1. Fix CSS Loading

**Option A**: Add react-complex-tree CSS to index.html

```html
<link rel="stylesheet" href="https://unpkg.com/react-complex-tree/lib/style-modern.css" />
```

**Option B**: Use inline styles only (no CSS classes)

```typescript
// Use style props instead of className
style={{ padding: '4px', backgroundColor: isSelected ? '#e0f2fe' : 'transparent' }}
```

#### 2. Fix Nested Buttons Issue

```typescript
// DON'T: Buttons inside the tree item (which is already a button)
// DO: Use divs with onClick handlers that stopPropagation
<div onClick={(e) => { e.stopPropagation(); handleAction(); }}>
```

#### 3. Fix Folder Expansion (ROOT CAUSE FOUND)

The folders won't expand because of TWO critical missing pieces:

**A. Missing `isFolder` property in TreeItem**

```typescript
// WRONG - Only hasChildren is not enough!
const item: TreeItem = {
  hasChildren: isDirectory,  // This alone doesn't work!
  children: childIds,
  ...
}

// CORRECT - Must include BOTH hasChildren AND isFolder
const item: TreeItem = {
  hasChildren: isDirectory,
  isFolder: isDirectory,  // CRITICAL: This enables expand/collapse UI
  children: isDirectory ? childIds : undefined,
  ...
}
```

**B. Wrong InteractionMode conflicts with button rendering**

```typescript
// WRONG - ClickItemToExpand doesn't work well with custom button rendering
interactionMode={InteractionMode.ClickItemToExpand}

// CORRECT - Use DoubleClick or ClickArrowToExpand
interactionMode={InteractionMode.ClickArrowToExpand}
// OR
interactionMode={InteractionMode.DoubleClick}
```

### Phase 1: Setup & Core Integration

#### 1. Install Dependencies

```bash
pnpm add react-complex-tree
```

#### 2. Create VFS Data Provider (`/src/components/filetree/VFSDataProvider.ts`)

```typescript
import { TreeDataProvider, TreeItem } from 'react-complex-tree';
import { getVFSService } from '../../services/vfs-service';

export class VFSDataProvider implements TreeDataProvider {
  private vfs = getVFSService();
  private cache = new Map<string, TreeItem[]>();

  async getTreeItem(itemId: string): Promise<TreeItem> {
    // Fetch item metadata from VFS
    // Return TreeItem with data: { name, path, type, size, modified }
  }

  async getTreeItems(itemIds: string[]): Promise<TreeItem[]> {
    // Batch fetch multiple items
  }

  async onRenameItem(item: TreeItem, name: string): Promise<void> {
    // Call VFS rename operation
    await this.vfs.writeFile(newPath, content);
    await this.vfs.deleteFile(oldPath);
  }

  async onChangeItemChildren(itemId: string, newChildren: string[]): Promise<void> {
    // Handle drag & drop file moves
  }

  // Custom method for delete
  async deleteItem(itemPath: string): Promise<void> {
    await this.vfs.deleteFile(itemPath);
  }
}
```

#### 3. Create FileTree Component (`/src/components/filetree/FileTree.tsx`)

```typescript
import {
  UncontrolledTreeEnvironment,
  Tree,
  TreeItemIndex,
  InteractionMode
} from 'react-complex-tree';
import { VFSDataProvider } from './VFSDataProvider';

interface FileTreeProps {
  rootPath?: string;
  onFileSelect?: (path: string) => void;
  onFileDelete?: (path: string) => void;
  showHidden?: boolean;
  className?: string;
}

export const FileTree: React.FC<FileTreeProps> = ({
  rootPath = '/',
  onFileSelect,
  onFileDelete,
  showHidden = false,
  className = ''
}) => {
  const [dataProvider] = useState(() => new VFSDataProvider());

  return (
    <UncontrolledTreeEnvironment
      dataProvider={dataProvider}
      getItemTitle={(item) => item.data.name}
      viewState={{}}
      canDragAndDrop={true}
      canDropOnFolder={true}
      canReorderItems={true}
      renderItemTitle={({ title, item, context }) => (
        <FileTreeNode
          item={item}
          onDelete={() => handleDelete(item)}
        />
      )}
      onPrimaryAction={(item) => {
        if (item.data.type === 'file') {
          onFileSelect?.(item.data.path);
        }
      }}
      interactionMode={InteractionMode.DoubleClick}
    >
      <Tree treeId="vfs-tree" rootItem={rootPath} treeLabel="Files" />
    </UncontrolledTreeEnvironment>
  );
};
```

#### 4. Create FileTreeNode Component (`/src/components/filetree/FileTreeNode.tsx`)

```typescript
import { Folder, FolderOpen, File, Trash2 } from 'lucide-react';

export const FileTreeNode = ({ item, onDelete }) => {
  const getIcon = () => {
    if (item.data.type === 'directory') {
      return item.isExpanded ? <FolderOpen /> : <Folder />;
    }
    return <File />;
  };

  return (
    <div className="flex items-center gap-2 group">
      <span className="w-4 h-4">{getIcon()}</span>
      <span className="flex-1">{item.data.name}</span>
      <button
        onClick={(e) => {
          e.stopPropagation();
          onDelete();
        }}
        className="opacity-0 group-hover:opacity-100"
      >
        <Trash2 className="w-3 h-3" />
      </button>
    </div>
  );
};
```

### Phase 2: Advanced Features

#### 5. Add Directory Watching

```typescript
// In VFSDataProvider
async watchDirectory(path: string) {
  const watchId = await this.vfs.watchDirectory(path, (changes) => {
    // Invalidate cache
    this.cache.delete(path);
    // Trigger tree refresh
    this.onDataChanged?.();
  });
  this.watchers.set(path, watchId);
}
```

#### 6. Add Context Menu

- Right-click menu for:
  - Delete
  - Rename
  - Create new file/folder
  - Copy path

#### 7. Add Search/Filter

```typescript
// Add search box above tree
<input
  type="text"
  placeholder="Search files..."
  onChange={(e) => setSearchQuery(e.target.value)}
/>

// Filter items in data provider based on search
```

### Phase 3: Replace Existing Component

#### 8. Update VFSManager.tsx

```typescript
import { FileTree } from './filetree/FileTree';

export const VFSManager = () => {
  return (
    <div className="flex h-full">
      <EditorSidebar title="File Browser">
        <FileTree
          onFileSelect={handleFileSelect}
          onFileDelete={handleFileDelete}
          showHidden={showHidden}
        />
      </EditorSidebar>
      {/* ... rest of component */}
    </div>
  );
};
```

#### 9. Remove Old VFSFileBrowser

- Delete `/src/components/VFSFileBrowser.tsx`
- Update any other imports

### Phase 4: Testing & Polish

#### 10. Test Cases

- [ ] Large directories (1000+ files)
- [ ] Deep nesting (10+ levels)
- [ ] File operations (create, delete, rename)
- [ ] Drag & drop between folders
- [ ] Hidden files toggle
- [ ] Search functionality
- [ ] Error handling (network issues, permissions)
- [ ] State persistence (expanded folders)

#### 11. Performance Optimizations

- Implement virtual scrolling for large lists
- Cache directory contents
- Debounce search input
- Lazy load deep directories

#### 12. Styling & UX

- Add loading spinners
- Error boundaries
- Empty state messages
- File type icons
- Size and date formatting

## Benefits Over Current Implementation

| Feature          | Current VFSFileBrowser | New FileTree             |
| ---------------- | ---------------------- | ------------------------ |
| State Management | Manual                 | Built-in                 |
| Accessibility    | Basic                  | W3C Compliant            |
| Multi-select     | ❌                     | ✅                       |
| Drag & Drop      | ❌                     | ✅                       |
| Keyboard Nav     | Basic                  | Full Support             |
| Performance      | OK                     | Optimized                |
| Mobile Support   | Basic                  | Better (but not perfect) |

## Migration Notes

1. The new component will be backward compatible with existing props
2. State will be managed internally by react-complex-tree
3. Custom styling can be applied via CSS classes
4. Context menus and actions remain customizable

## Future Enhancements

- [ ] File preview on hover
- [ ] Breadcrumb navigation
- [ ] File upload via drag & drop
- [ ] Git status indicators
- [ ] File permissions display
- [ ] Batch operations (select multiple files)
- [ ] Undo/redo for file operations

## Resources

- [react-complex-tree docs](https://rct.lukasbach.com/)
- [GitHub repo](https://github.com/lukasbach/react-complex-tree)
- [Examples](https://rct.lukasbach.com/storybook)
