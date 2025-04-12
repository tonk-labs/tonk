import { create } from "zustand";
import { ls, platformSensitiveJoin, readBinary } from "../ipc/files";
import { useConfigStore } from "./configStore";
import { TreeItems, TreeItem, FileType } from "../components/Tree";
import * as Automerge from "@automerge/automerge";
import * as AutomergeWasm from "@automerge/automerge-wasm";
import debounce from "lodash.debounce";
import { FileChangeEvent } from "../types";
import { runServer } from "../ipc/hub";

// Initialize Automerge with WASM for Electron environment
Automerge.use(AutomergeWasm);

interface ProjectState {
  items: TreeItems;
  isLoading: boolean;
  error: string | null;
  selectedItem: TreeItem | null;
  automergeContent: string | null;
  changedParents: string[];
  loadProjects: () => Promise<void>;
  setSelectedItem: (item: TreeItem | null) => void;
  inspectAutomergeFile: (path: string) => Promise<void>;
  startFileWatching: () => Promise<void>;
  stopFileWatching: () => Promise<void>;
  handleFileChange: (event: FileChangeEvent) => Promise<void>;
}

const REQUIRED_DIRECTORIES = ["apps", "stores"];

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
  homePath: string,
): Promise<string | null> => {
  try {
    const contents = await ls(homePath);
    if (!contents) {
      return "Could not read home directory";
    }

    const existingDirs = new Set(
      contents.filter((item) => item.isDirectory).map((item) => item.name),
    );

    const missingDirs = REQUIRED_DIRECTORIES.filter(
      (dir) => !existingDirs.has(dir),
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
  children: string[] = [],
): TreeItem => ({
  index,
  isFolder,
  children,
  data: {
    name,
    fileType,
  },
});

const getFileType = (path: string): FileType => {
  const segments = path.split("/");
  const topLevel = segments[0];

  // Map the top-level directory to the appropriate file type
  switch (topLevel) {
    case "apps":
      return FileType.App;
    case "stores":
      return FileType.Store;
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
    REQUIRED_DIRECTORIES,
  ),
  apps: createTreeItem("apps", "apps", FileType.Section, true, []),
  stores: createTreeItem("stores", "stores", FileType.Section, true, []),
});

const processDirectoryContents = async (
  dirPath: string,
  dir: string,
  items: TreeItems,
  sectionType: FileType,
): Promise<void> => {
  const contents = await ls(dirPath);
  if (!contents) return;

  for (const item of contents) {
    // Skip blacklisted files
    if (shouldIgnoreFile(item.name)) {
      continue;
    }

    // For stores directory, only include files ending with .automerge
    if (
      dir === "stores" &&
      ((!item.isDirectory && !item.name.endsWith(".automerge")) ||
        item.isDirectory)
    ) {
      continue;
    }

    const itemPath = await platformSensitiveJoin([dir, item.name]);
    const itemId = itemPath!.replace(/\//g, "/");

    items[itemId] = createTreeItem(
      itemId,
      item.name,
      sectionType, // Use the section type for all items in this directory
      false,
      // item.isDirectory,
      [],
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
    const sectionType = getFileType(dir);
    await processDirectoryContents(dirPath!, dir, items, sectionType);
  }

  return items;
};

const findChangedParents = (
  oldItems: TreeItems,
  newItems: TreeItems,
): string[] => {
  const changedParents = new Set<string>();

  // Helper to get sorted children array for comparison
  const getChildrenString = (item: TreeItem) =>
    [...item.children].sort().join(",");

  // Compare each item's children
  Object.keys(newItems).forEach((itemId) => {
    const newItem = newItems[itemId];
    const oldItem = oldItems[itemId];

    // If item exists in both and has different children, add it to changed parents
    if (oldItem && newItem) {
      const oldChildrenStr = getChildrenString(oldItem);
      const newChildrenStr = getChildrenString(newItem);

      if (oldChildrenStr !== newChildrenStr) {
        changedParents.add(newItem.data.name);
      }
    }
  });

  return Array.from(changedParents);
};

const useProjectStore = create<ProjectState>((set, get) => ({
  items: {},
  isLoading: false,
  error: null,
  selectedItem: null,
  automergeContent: null,
  changedParents: [],

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

      // Start file watching after initial load
      await get().startFileWatching();
    } catch (error) {
      console.error("Failed to load projects:", error);
      set({
        error:
          error instanceof Error ? error.message : "Failed to load projects",
        isLoading: false,
      });
    }
  },

  setSelectedItem: async (item: TreeItem | null) => {
    set({ selectedItem: item });

    if (
      item &&
      item.index.startsWith("stores/") &&
      item.index.endsWith(".automerge") &&
      !item.isFolder
    ) {
      await get().inspectAutomergeFile(item.index);
    } else {
      set({ automergeContent: null });
    }
  },

  inspectAutomergeFile: async (path: string) => {
    try {
      const config = useConfigStore.getState().config;
      if (!config?.homePath) {
        throw new Error("Home path not configured");
      }

      // Construct the full path
      const fullPath = await platformSensitiveJoin([config.homePath, path]);
      if (!fullPath) {
        throw new Error("Failed to construct file path");
      }

      // Use the IPC readFile function
      const fileContent = await readBinary(fullPath);

      // Handle case when file content is not available
      if (!fileContent) {
        // Set a user-friendly message
        set({
          automergeContent: null,
          error: "The file is being processed. Please try again in a moment.",
        });
        return;
      }

      // Load the Automerge document
      const doc = Automerge.load(fileContent);

      const content = JSON.stringify(doc, null, 2);
      set({ automergeContent: content, error: null });
    } catch (error) {
      console.error("Failed to inspect Automerge file:", error);

      // Provide more helpful error messages based on the error type
      let errorMessage = "Failed to read Automerge document";

      if (error instanceof Error) {
        const message = error.message;

        if (message.includes("initializing") || message.includes("empty")) {
          errorMessage =
            "The file is still being created. Please try again in a moment.";
        } else if (
          message.includes("locked") ||
          message.includes("being written")
        ) {
          errorMessage =
            "The file is currently being modified. Please try again in a moment.";
        } else if (message.includes("Failed to read file")) {
          errorMessage =
            "The file could not be read. It may still be initializing.";
        } else {
          errorMessage = `${errorMessage}: ${message}`;
        }
      }

      set({
        automergeContent: null,
        error: errorMessage,
      });
    }
  },

  startFileWatching: async () => {
    try {
      const config = useConfigStore.getState().config;
      if (!config?.homePath) {
        throw new Error("Home path not configured");
      }

      // Start file watching
      await window.electronAPI.startFileWatching();

      // Add event listener for file changes
      window.addEventListener("file-change", async (event: Event) => {
        const customEvent = event as CustomEvent<FileChangeEvent>;
        console.log(customEvent);
        await get().handleFileChange(customEvent.detail);
      });
    } catch (error) {
      console.error("Failed to start file watching:", error);
      set({
        error: `Failed to start file watching: ${error instanceof Error ? error.message : "Unknown error"}`,
      });
    }
  },

  stopFileWatching: async () => {
    try {
      await window.electronAPI.stopFileWatching();
      window.removeEventListener("file-change", async (event: Event) => {
        const customEvent = event as CustomEvent<FileChangeEvent>;
        await get().handleFileChange(customEvent.detail);
      });
    } catch (error) {
      console.error("Failed to stop file watching:", error);
    }
  },

  handleFileChange: async (event: FileChangeEvent) => {
    try {
      const config = useConfigStore.getState().config;
      if (!config?.homePath) {
        throw new Error("Home path not configured");
      }

      if (event.type === "addDir") {
        debounce(async () => await runServer(true), 1000);
      }

      const oldItems = { ...get().items };
      // Reload the entire project structure when files change
      const items = await loadProjectStructure(config.homePath);

      // Find parents with changed children
      const changedParents = findChangedParents(oldItems, items);

      set({
        items: {
          ...items,
        },
        changedParents,
      });

      // If the changed file is the currently selected file and it's an automerge file in the stores directory, reload its content
      const selectedItem = get().selectedItem;
      if (
        selectedItem &&
        event.path.includes(selectedItem.index) &&
        selectedItem.index.startsWith("stores/") &&
        selectedItem.index.endsWith(".automerge") &&
        !selectedItem.isFolder
      ) {
        await get().inspectAutomergeFile(selectedItem.index);
      }
    } catch (error) {
      console.error("Failed to handle file change:", error);
      set({
        error: `Failed to handle file change: ${error instanceof Error ? error.message : "Unknown error"}`,
      });
    }
  },
}));

export { useProjectStore };
