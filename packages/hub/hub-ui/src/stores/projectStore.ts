import { create } from "zustand";
import { ls, platformSensitiveJoin } from "../ipc/files";
import { useConfigStore } from "./configStore";
import { TreeItems, TreeItem, FileType } from "../components/Tree";

interface ProjectState {
  items: TreeItems;
  isLoading: boolean;
  error: string | null;
  selectedItem: TreeItem | null;
  loadProjects: () => Promise<void>;
  setSelectedItem: (item: TreeItem | null) => void;
}

const REQUIRED_DIRECTORIES = ["apps", "stores", "integrations", "data"];

// Blacklist of files and patterns to ignore (similar to .gitignore)
const BLACKLISTED_FILES = [
  ".DS_Store",
  "Thumbs.db", // Windows thumbnail cache
  "*.log", // Log files
  ".env", // Environment files
  ".env.local",
  ".env.*.local",
  "node_modules", // Dependencies
  "npm-debug.log*",
  "yarn-debug.log*",
  "yarn-error.log*",
];

// Helper function to check if a file should be ignored
const shouldIgnoreFile = (filename: string): boolean => {
  return BLACKLISTED_FILES.some((pattern) => {
    if (pattern.includes("*")) {
      // Convert glob pattern to regex
      const regexPattern = pattern.replace(/\./g, "\\.").replace(/\*/g, ".*");
      return new RegExp(`^${regexPattern}$`).test(filename);
    }
    return filename === pattern;
  });
};

const validateProjectStructure = async (
  homePath: string
): Promise<string | null> => {
  try {
    const contents = await ls(homePath);
    if (!contents) {
      return "Could not read home directory";
    }

    const existingDirs = new Set(
      contents.filter((item) => item.isDirectory).map((item) => item.name)
    );

    const missingDirs = REQUIRED_DIRECTORIES.filter(
      (dir) => !existingDirs.has(dir)
    );

    if (missingDirs.length > 0) {
      return `Missing required directories: ${missingDirs.join(", ")}`;
    }

    return null;
  } catch (error) {
    return `Failed to validate project structure: ${error instanceof Error ? error.message : "Unknown error"}`;
  }
};

const createTreeItem = (
  index: string,
  name: string,
  fileType: FileType,
  isFolder = false,
  children: string[] = []
): TreeItem => ({
  index,
  isFolder,
  children,
  data: {
    name,
    fileType,
  },
});

const getFileType = (path: string, isDirectory: boolean): FileType => {
  const segments = path.split("/");
  const topLevel = segments[0];

  // Map the top-level directory to the appropriate file type
  switch (topLevel) {
    case "apps":
      return FileType.App;
    case "stores":
      return FileType.Store;
    case "integrations":
      return FileType.Integration;
    case "data":
      return FileType.Data;
    case "root":
      return FileType.Section;
    default:
      // If we can't determine the type, default to Section
      return FileType.Section;
  }
};

const initializeBaseTreeItems = (): TreeItems => ({
  root: createTreeItem(
    "root",
    "root",
    FileType.Section,
    true,
    REQUIRED_DIRECTORIES
  ),
  apps: createTreeItem("apps", "apps", FileType.Section, true, []),
  stores: createTreeItem("stores", "stores", FileType.Section, true, []),
  integrations: createTreeItem(
    "integrations",
    "integrations",
    FileType.Section,
    true,
    []
  ),
  data: createTreeItem("data", "data", FileType.Section, true, []),
});

const processSubContents = async (
  parentPath: string,
  parentId: string,
  items: TreeItems,
  sectionType: FileType
): Promise<void> => {
  const subContents = await ls(parentPath);
  if (!subContents) return;

  for (const subItem of subContents) {
    // Skip blacklisted files
    if (shouldIgnoreFile(subItem.name)) {
      continue;
    }

    const subItemPath = await platformSensitiveJoin([parentId, subItem.name]);
    const subItemId = subItemPath!.replace(/\//g, "/");

    items[subItemId] = createTreeItem(
      subItemId,
      subItem.name,
      sectionType, // Use the parent section's type for all children
      subItem.isDirectory,
      []
    );

    items[parentId].children.push(subItemId);
  }
};

const processDirectoryContents = async (
  dirPath: string,
  dir: string,
  items: TreeItems,
  sectionType: FileType
): Promise<void> => {
  const contents = await ls(dirPath);
  if (!contents) return;

  for (const item of contents) {
    const itemPath = await platformSensitiveJoin([dir, item.name]);
    const itemId = itemPath!.replace(/\//g, "/");

    if (shouldIgnoreFile(item.name)) {
      continue;
    }

    items[itemId] = createTreeItem(
      itemId,
      item.name,
      sectionType, // Use the section type for all items in this directory
      false,
      // item.isDirectory,
      []
    );

    // Add to parent's children
    items[dir].children.push(itemId);

    // If it's a directory, process its contents
    // For now we only go one level deep
    // if (item.isDirectory) {
    //   const subDir = await platformSensitiveJoin([dirPath, item.name]);
    //   await processSubContents(subDir!, itemId, items, sectionType);
    // }
  }
};

const loadProjectStructure = async (homePath: string): Promise<TreeItems> => {
  const items = initializeBaseTreeItems();

  // Process each required directory
  for (const dir of REQUIRED_DIRECTORIES) {
    const dirPath = await platformSensitiveJoin([homePath, dir]);
    // Determine the section type based on the directory
    const sectionType = getFileType(dir, true);
    await processDirectoryContents(dirPath!, dir, items, sectionType);
  }

  return items;
};

export const useProjectStore = create<ProjectState>((set) => ({
  items: {},
  isLoading: false,
  error: null,
  selectedItem: null,
  loadProjects: async () => {
    set({ isLoading: true, error: null });
    try {
      const config = useConfigStore.getState().config;
      if (!config?.homePath) {
        throw new Error("Home path not configured");
      }

      // Validate project structure first
      const validationError = await validateProjectStructure(config.homePath);
      if (validationError) {
        throw new Error(validationError);
      }

      const items = await loadProjectStructure(config.homePath);
      set({ items, isLoading: false });
    } catch (error) {
      console.error("Failed to load projects:", error);
      set({
        error:
          error instanceof Error ? error.message : "Failed to load projects",
        isLoading: false,
      });
    }
  },
  setSelectedItem: (item: TreeItem | null) => set({ selectedItem: item }),
}));
