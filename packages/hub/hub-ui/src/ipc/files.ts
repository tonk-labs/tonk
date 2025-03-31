export const ls = async (dirPath: string) => {
  try {
    // Launch the app with the selected docId
    return await window.electronAPI.ls(dirPath);
  } catch (error: unknown) {
    console.error("Error reading directory:", error);
    if (error instanceof Error) {
      console.error("Error reading directory: " + error.message);
    } else {
      console.error("An unexpected error occurred while reading directories");
    }
  }
};

export const readFile = async (filePath: string) => {
  try {
    // Launch the app with the selected docId
    return await window.electronAPI.readFile(filePath);
  } catch (error: unknown) {
    console.error("Error reading file:", error);
    if (error instanceof Error) {
      console.error("Error reading file: " + error.message);
    } else {
      console.error("An unexpected error occurred while reading file");
    }
  }
};

export const writeFile = async (filePath: string, content: string) => {
  try {
    // Launch the app with the selected docId
    return await window.electronAPI.writeFile(filePath, content);
  } catch (error: unknown) {
    console.error("Error writing file:", error);
    if (error instanceof Error) {
      console.error("Error writing file: " + error.message);
    } else {
      console.error("An unexpected error occurred while writing file");
    }
  }
};

export const platformSensitiveJoin = async (paths: string[]) => {
  try {
    // Launch the app with the selected docId
    return await window.electronAPI.platformSensitiveJoin(paths);
  } catch (error: unknown) {
    console.error("Error joining paths:", error);
    if (error instanceof Error) {
      console.error("Error joining paths: " + error.message);
    } else {
      console.error("An unexpected error occurred joining paths");
    }
  }
};
