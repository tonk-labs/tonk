import { TreeDataProvider, TreeItem } from 'react-complex-tree';
import { getVFSService } from '../../services/vfs-service';

export interface VFSTreeItemData {
  name: string;
  path: string;
  type: 'file' | 'directory';
  size?: number;
  modified?: Date;
}

export class VFSDataProvider implements TreeDataProvider {
  private vfs = getVFSService();
  private cache = new Map<string, TreeItem<VFSTreeItemData>>();
  private childrenCache = new Map<string, string[]>();
  private onDataChangedCallback?: () => void;
  private showHidden = false;
  private rootPath = '/';
  private searchQuery = '';
  private watchHandles = new Map<string, string>();

  constructor(rootPath = '/') {
    this.rootPath = rootPath;
    // Bind methods to preserve context
    this.getTreeItem = this.getTreeItem.bind(this);
    this.getTreeItems = this.getTreeItems.bind(this);
    this.onRenameItem = this.onRenameItem.bind(this);
    this.onChangeItemChildren = this.onChangeItemChildren.bind(this);

    // Start watching the root directory
    this.watchDirectory(rootPath);
  }

  setOnDataChanged(callback: () => void) {
    this.onDataChangedCallback = callback;
  }

  setShowHidden(show: boolean) {
    if (this.showHidden !== show) {
      this.showHidden = show;
      this.refresh();
    }
  }

  setSearchQuery(query: string) {
    if (this.searchQuery !== query) {
      this.searchQuery = query;
      this.notifyDataChanged();
    }
  }

  private notifyDataChanged() {
    if (this.onDataChangedCallback) {
      this.onDataChangedCallback();
    }
  }

  private normalizePath(path: string): string {
    if (path === 'root') return '/';
    return path.startsWith('/') ? path : `/${path}`;
  }

  private getItemId(path: string): string {
    return path === this.rootPath ? 'root' : path;
  }

  async getTreeItem(itemId: string): Promise<TreeItem<VFSTreeItemData>> {
    const path = this.normalizePath(itemId);

    // Check cache first
    if (this.cache.has(itemId)) {
      return this.cache.get(itemId)!;
    }

    try {
      // For root directory (could be custom root path)
      if (itemId === 'root') {
        const actualPath = this.rootPath;
        const children = await this.loadDirectoryChildren(actualPath);
        const rootName = this.rootPath === '/' ? 'Root' : this.rootPath.split('/').pop() || 'Root';
        const item: TreeItem<VFSTreeItemData> = {
          index: 'root',
          canMove: false,
          canRename: false,
          hasChildren: children.length > 0,
          children: children.map(child => this.getItemId(child)),
          data: {
            name: rootName,
            path: actualPath,
            type: 'directory'
          }
        };
        this.cache.set('root', item);
        this.childrenCache.set(actualPath, children);

        // Watch this directory
        this.watchDirectory(actualPath);

        return item;
      }

      // Check if file/directory exists
      const exists = await this.vfs.exists(path);
      if (!exists) {
        throw new Error(`Path ${path} does not exist`);
      }

      // Extract name from path
      const name = path.split('/').pop() || path;

      // Determine if it's a directory by trying to list it
      let isDirectory = false;
      let children: string[] = [];

      try {
        children = await this.loadDirectoryChildren(path);
        if (children.length > 0) {
        isDirectory = true;
        }
      } catch {
        // If listing fails, it's likely a file
        isDirectory = false;
      }

      const item: TreeItem<VFSTreeItemData> = {
        index: itemId,
        canMove: true,
        canRename: true,
        isFolder: isDirectory,
        children: isDirectory ? children.map(child => this.getItemId(child)) : undefined,
        data: {
          name,
          path,
          type: isDirectory ? 'directory' : 'file'
        }
      };

      this.cache.set(itemId, item);
      if (isDirectory) {
        this.childrenCache.set(path, children);
        // Watch this directory for changes
        this.watchDirectory(path);
      }

      return item;
    } catch (error) {
      console.error(`Failed to get tree item for ${path}:`, error);
      // Return a placeholder item for errors
      const name = path.split('/').pop() || path;
      const item: TreeItem<VFSTreeItemData> = {
        index: itemId,
        canMove: false,
        canRename: false,
        isFolder: false,
        data: {
          name,
          path,
          type: 'file'
        }
      };
      return item;
    }
  }

  async getTreeItems(itemIds: string[]): Promise<TreeItem<VFSTreeItemData>[]> {
    const items = await Promise.all(
      itemIds.map(async (id) => {
        try {
          return await this.getTreeItem(id);
        } catch (error) {
          console.error(`Failed to get item ${id}:`, error);
          return null;
        }
      })
    );

    return items.filter((item): item is TreeItem<VFSTreeItemData> => item !== null);
  }

  private async loadDirectoryChildren(path: string): Promise<string[]> {
    try {
      const items = await this.vfs.listDirectory(path);
      console.log(`Directory listing for ${path}:`, items);

      // Deduplicate items based on name
      const seen = new Set<string>();
      const uniqueItems = items.filter((item: any) => {
        const name = typeof item === 'string' ? item : (item?.name || item?.path?.split('/').pop());
        if (seen.has(name)) {
          return false;
        }
        seen.add(name);
        return true;
      });

      // Process items to get their full paths
      let children = uniqueItems.map((item: any) => {
        if (typeof item === 'string') {
          return path === '/' ? `/${item}` : `${path}/${item}`;
        } else if (item && typeof item === 'object') {
          const name = item.name || item.path?.split('/').pop() || 'unknown';
          return path === '/' ? `/${name}` : `${path}/${name}`;
        }
        return null;
      }).filter((path): path is string => {
        if (path === null) return false;
        const name = path.split('/').pop() || '';

        // Filter by hidden files
        if (!this.showHidden && name.startsWith('.')) {
          return false;
        }

        // Filter by search query
        if (this.searchQuery && this.searchQuery.trim()) {
          return name.toLowerCase().includes(this.searchQuery.toLowerCase());
        }

        return true;
      });

      return children;
    } catch (error) {
      console.error(`Failed to load directory ${path}:`, error);
      return [];
    }
  }

  async onRenameItem(item: TreeItem<VFSTreeItemData>, name: string): Promise<void> {
    const oldPath = item.data.path;
    const parentPath = oldPath.substring(0, oldPath.lastIndexOf('/')) || '/';
    const newPath = parentPath === '/' ? `/${name}` : `${parentPath}/${name}`;

    try {
      // Read the current content
      const content = await this.vfs.readFile(oldPath);

      // Write to new path
      await this.vfs.writeFile(newPath, content, true);

      // Delete old path
      await this.vfs.deleteFile(oldPath);

      // Clear cache entries for affected paths
      this.invalidateCache(oldPath);
      this.invalidateCache(newPath);
      this.invalidateCache(parentPath);

      this.notifyDataChanged();
    } catch (error) {
      console.error(`Failed to rename ${oldPath} to ${newPath}:`, error);
      throw error;
    }
  }

  async onChangeItemChildren(itemId: string, newChildren: string[]): Promise<void> {
    // This would handle drag & drop operations
    // For now, we'll just update the cache
    const path = this.normalizePath(itemId);
    const childPaths = newChildren.map(childId => this.normalizePath(childId));
    this.childrenCache.set(path, childPaths);

    // Clear cache for the item to force refresh
    this.cache.delete(itemId);
    this.notifyDataChanged();
  }

  // Custom methods for additional operations
  async deleteItem(path: string): Promise<void> {
    try {
      await this.vfs.deleteFile(path);
      this.invalidateCache(path);

      // Also invalidate parent directory
      const parentPath = path.substring(0, path.lastIndexOf('/')) || '/';
      this.invalidateCache(parentPath);

      this.notifyDataChanged();
    } catch (error) {
      console.error(`Failed to delete ${path}:`, error);
      throw error;
    }
  }

  async createFile(parentPath: string, fileName: string, content: string = ''): Promise<void> {
    const filePath = parentPath === '/' ? `/${fileName}` : `${parentPath}/${fileName}`;

    try {
      await this.vfs.writeFile(filePath, { content }, true);
      this.invalidateCache(parentPath);
      this.notifyDataChanged();
    } catch (error) {
      console.error(`Failed to create file ${filePath}:`, error);
      throw error;
    }
  }

  async createDirectory(parentPath: string, dirName: string): Promise<void> {
    const dirPath = parentPath === '/' ? `/${dirName}` : `${parentPath}/${dirName}`;

    try {
      // Create a placeholder file to establish the directory
      const placeholderPath = `${dirPath}/.gitkeep`;
      await this.vfs.writeFile(placeholderPath, { content: '' }, true);
      this.invalidateCache(parentPath);
      this.invalidateCache(dirPath);
      this.notifyDataChanged();
    } catch (error) {
      console.error(`Failed to create directory ${dirPath}:`, error);
      throw error;
    }
  }

  private invalidateCache(path: string): void {
    const itemId = this.getItemId(path);
    this.cache.delete(itemId);
    this.childrenCache.delete(path);

    // Also invalidate any cached children
    for (const [cachedPath, cachedItem] of this.cache.entries()) {
      if (cachedItem.data.path.startsWith(path + '/')) {
        this.cache.delete(cachedPath);
      }
    }

    for (const cachedPath of this.childrenCache.keys()) {
      if (cachedPath.startsWith(path + '/')) {
        this.childrenCache.delete(cachedPath);
      }
    }
  }

  // Method to refresh the entire tree
  refresh(): void {
    this.cache.clear();
    this.childrenCache.clear();
    this.notifyDataChanged();
  }

  // Watch a directory for changes
  private async watchDirectory(path: string): Promise<void> {
    try {
      // If already watching this directory, skip
      if (this.watchHandles.has(path)) {
        return;
      }

      const watchId = await this.vfs.watchDirectory(path, (changeData) => {
        console.log(`Directory ${path} changed:`, changeData);

        // Clear cache for the changed directory
        this.invalidateCache(path);

        // Notify that data has changed
        this.notifyDataChanged();
      });

      this.watchHandles.set(path, watchId);
    } catch (error) {
      console.error(`Failed to watch directory ${path}:`, error);
    }
  }

  // Clean up watchers
  async cleanup(): Promise<void> {
    for (const [path, watchId] of this.watchHandles) {
      try {
        await this.vfs.unwatchDirectory(watchId);
      } catch (error) {
        console.error(`Failed to unwatch directory ${path}:`, error);
      }
    }
    this.watchHandles.clear();
  }
}