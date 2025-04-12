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

export const readBinary = async (
  filePath: string,
): Promise<Uint8Array | undefined> => {
  try {
    return await window.electronAPI.readBinary(filePath);
  } catch (error: unknown) {
    // Log the error for debugging
    console.error("Error reading binary:", error);

    // Check for specific error conditions
    if (error instanceof Error) {
      const errorMessage = error.message;

      // Handle specific error cases
      if (
        errorMessage.includes("empty") ||
        errorMessage.includes("initializing")
      ) {
        console.warn(
          `File at ${filePath} exists but may still be initializing`,
        );
      } else if (
        errorMessage.includes("locked") ||
        errorMessage.includes("being written")
      ) {
        console.warn(
          `File at ${filePath} is currently locked or being written to`,
        );
      } else {
        console.error("Error reading binary: " + errorMessage);
      }
    } else {
      console.error("An unexpected error occurred while reading binary");
    }

    // Return undefined to indicate failure
    return undefined;
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
